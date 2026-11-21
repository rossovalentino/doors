extern crate std;
use super::engine::{self, Request, Session, Ui, level, msg};
use super::execute::{Target, code, packed_guid, records, unpacked_guid};
use super::install::{Machine, RegValue, Root};
use super::link::Link;
use super::*;
use alloc::{collections::BTreeMap, format, string::ToString, vec};

const SAMPLE: &[u8] = include_bytes!("../testdata/sample.msi");

/// A file system and registry in memory (paths compared case-insensitively).
#[derive(Clone, Default)]
struct Memory {
    files: BTreeMap<String, Vec<u8>>,
    /// (created, modified) seconds.
    times: BTreeMap<String, (u64, u64)>,
    folders: Vec<String>,
    /// (key, name) -> (name as written, value).
    registry: BTreeMap<(String, String), (String, RegValue)>,
    keys: Vec<String>,
    log: Vec<String>,
    /// Programs run (EXE custom actions).
    ran: Vec<String>,
    free: Option<u64>,
    boot: u64,
    clock: u64,
    /// The machine as it was when a program asked to sleep (the crash).
    crash: Option<std::boxed::Box<Memory>>,
    services: Vec<String>,
}
/// A key in its view: the 32-bit view's keys apart (the test machine does
/// not model which keys WOW64 shares).
fn key(root: Root, key: &str) -> String {
    let view = if root.view32() { " (32-bit)" } else { "" };
    format!("{}{view}\\{}", root.name(), key).to_ascii_lowercase()
}
impl Memory {
    fn machine() -> Self {
        let mut m = Memory::default();
        for f in [
            "C:\\Program Files",
            "C:\\Program Files\\Common Files",
            "C:\\ProgramData",
            "C:\\ProgramData\\Microsoft",
            "C:\\ProgramData\\Microsoft\\Windows",
            "C:\\ProgramData\\Microsoft\\Windows\\Start Menu",
            "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs",
            "C:\\Windows",
            "C:\\Windows\\Installer",
            "C:\\Users",
            "C:\\Users\\Ada",
            "C:\\Users\\Ada\\Downloads",
            "C:\\Users\\Ada\\AppData",
            "C:\\Users\\Ada\\AppData\\Local",
            "C:\\Users\\Ada\\AppData\\Local\\Temp",
        ] {
            m.folders.push(f.to_ascii_lowercase());
        }
        m
    }
    fn file(&self, path: &str) -> Option<&Vec<u8>> {
        self.files.get(&path.to_ascii_lowercase())
    }
    fn get(&self, root: Root, k: &str, name: &str) -> Option<RegValue> {
        self.registry
            .get(&(key(root, k), name.to_ascii_lowercase()))
            .map(|v| v.1.clone())
    }
    fn tick(&mut self) -> u64 {
        self.clock += 10;
        self.clock
    }
}
impl Target for Memory {
    fn create_folder(&mut self, path: &str) -> Result<(), u32> {
        let p = path.to_ascii_lowercase();
        let parent = p
            .rsplit_once('\\')
            .map(|x| x.0.to_string())
            .unwrap_or_default();
        if !parent.ends_with(':') && !self.folders.contains(&parent) {
            return Err(3);
        }
        if !self.folders.contains(&p) {
            self.folders.push(p);
        }
        Ok(())
    }
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32> {
        let parent = path
            .rsplit_once('\\')
            .map(|p| p.0.to_ascii_lowercase())
            .unwrap();
        if !self.folders.contains(&parent) {
            return Err(3); // ERROR_PATH_NOT_FOUND
        }
        let t = self.tick();
        let p = path.to_ascii_lowercase();
        let created = self.times.get(&p).map_or(t, |x| x.0);
        self.times.insert(p.clone(), (created, t));
        self.files.insert(p, data.to_vec());
        Ok(())
    }
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, u32> {
        self.files.get(&path.to_ascii_lowercase()).cloned().ok_or(2)
    }
    fn exists(&mut self, path: &str) -> bool {
        let p = path.to_ascii_lowercase();
        self.files.contains_key(&p) || self.folders.contains(&p)
    }
    fn remove_file(&mut self, path: &str) -> Result<(), u32> {
        let p = path.to_ascii_lowercase();
        self.times.remove(&p);
        self.files.remove(&p).map(|_| ()).ok_or(2)
    }
    fn remove_folder(&mut self, path: &str) -> Result<(), u32> {
        let p = path.to_ascii_lowercase();
        let prefix = format!("{p}\\");
        if self.files.keys().any(|f| f.starts_with(&prefix))
            || self.folders.iter().any(|f| f.starts_with(&prefix))
        {
            return Err(145); // ERROR_DIR_NOT_EMPTY
        }
        if !self.folders.contains(&p) {
            return Err(2);
        }
        self.folders.retain(|f| *f != p);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        let f = from.to_ascii_lowercase();
        let data = self.files.remove(&f).ok_or(2u32)?;
        let times = self.times.remove(&f);
        let t = to.to_ascii_lowercase();
        self.files.insert(t.clone(), data);
        if let Some(times) = times {
            self.times.insert(t, times);
        }
        Ok(())
    }
    fn list(&mut self, folder: &str) -> Vec<String> {
        let prefix = format!("{}\\", folder.to_ascii_lowercase());
        self.files
            .keys()
            .filter_map(|f| f.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('\\'))
            .map(String::from)
            .collect()
    }
    fn file_times(&mut self, path: &str) -> Option<(u64, u64)> {
        self.times.get(&path.to_ascii_lowercase()).copied()
    }
    fn stamp(&mut self, path: &str) {
        if let Some(t) = self.times.get_mut(&path.to_ascii_lowercase()) {
            t.0 = t.1;
        }
    }
    fn free_bytes(&mut self, _path: &str) -> Option<u64> {
        self.free
    }
    fn boot(&mut self) -> u64 {
        self.boot
    }
    fn today(&mut self) -> Option<String> {
        Some("20261108".to_string())
    }
    fn set_value(
        &mut self,
        root: Root,
        k: &str,
        name: Option<&str>,
        value: &RegValue,
    ) -> Result<(), u32> {
        // The Doors registry's limits: 96-unit key paths, 48-unit names.
        if k.encode_utf16().count() > 96
            || name.is_some_and(|n| n.encode_utf16().count() > records::VALUE_NAME_UNITS)
        {
            return Err(87);
        }
        let full = key(root, k);
        if !self.keys.contains(&full) {
            self.keys.push(full.clone());
        }
        let n = name.unwrap_or("");
        self.registry.insert(
            (full, n.to_ascii_lowercase()),
            (n.to_string(), value.clone()),
        );
        Ok(())
    }
    fn create_key(&mut self, root: Root, k: &str) -> Result<(), u32> {
        let full = key(root, k);
        if !self.keys.contains(&full) {
            self.keys.push(full);
        }
        Ok(())
    }
    fn value(&mut self, root: Root, k: &str, name: &str) -> Option<RegValue> {
        self.get(root, k, name)
    }
    fn values(&mut self, root: Root, k: &str) -> Vec<(String, RegValue)> {
        let full = key(root, k);
        self.registry
            .iter()
            .filter(|((key, _), _)| *key == full)
            .map(|(_, v)| v.clone())
            .collect()
    }
    fn subkeys(&mut self, root: Root, k: &str) -> Vec<String> {
        let prefix = format!("{}\\", key(root, k));
        let mut out: Vec<String> = Vec::new();
        for key in &self.keys {
            if let Some(rest) = key.strip_prefix(&prefix) {
                let child = rest.split('\\').next().unwrap().to_string();
                if !out.contains(&child) {
                    out.push(child);
                }
            }
        }
        out
    }
    fn delete_value(&mut self, root: Root, k: &str, name: Option<&str>) {
        self.registry
            .remove(&(key(root, k), name.unwrap_or("").to_ascii_lowercase()));
    }
    fn delete_key(&mut self, root: Root, k: &str) {
        let k = key(root, k);
        let prefix = format!("{k}\\");
        self.registry
            .retain(|(key, _), _| *key != k && !key.starts_with(&prefix));
        self.keys
            .retain(|key| *key != k && !key.starts_with(&prefix));
    }
    /// caprobe.dll's CheckMachine: it reads ProductName and sets
    /// DOORS_CA_CHECK.
    fn call_dll(&mut self, path: &str, entry: &str, state: &mut engine::State) -> Result<u32, u32> {
        if entry != "CheckMachine" || self.file(path).is_none() {
            return Err(126);
        }
        if state.property("ProductName") != "Doors Tools" {
            return Ok(code::INSTALL_FAILURE);
        }
        state.set_property("DOORS_CA_CHECK", "passed");
        Ok(0)
    }
    fn run(&mut self, command_line: &str, _directory: &str, _wait: bool) -> Result<u32, u32> {
        self.ran.push(command_line.to_string());
        let words = engine::split_command_line(command_line);
        match words.get(1).map(String::as_str) {
            Some("mark") => {
                let path = words.get(2).cloned().unwrap_or_default();
                if let Some(i) = path.rfind('\\') {
                    let _ = self.create_folder(&path[..i]);
                }
                self.write_file(&path, b"installed").map(|()| 0)
            }
            Some("unmark") => {
                let path = words.get(2).cloned().unwrap_or_default();
                let _ = self.remove_file(&path);
                Ok(0)
            }
            Some("sleep") => {
                // The machine stops here: what the disk holds now is what
                // the next boot finds.
                let mut snapshot = self.clone();
                snapshot.crash = None;
                self.crash = Some(std::boxed::Box::new(snapshot));
                Ok(0)
            }
            Some("fail") => Ok(1),
            _ => Ok(0),
        }
    }
    fn install_service(&mut self, service: &install::ServicePlan) -> Result<(), u32> {
        self.services.push(service.name.clone());
        Ok(())
    }
    fn delete_service(&mut self, name: &str) -> Result<(), u32> {
        self.services.retain(|s| s != name);
        Ok(())
    }
    fn service_exists(&mut self, name: &str) -> bool {
        self.services.iter().any(|s| s == name)
    }
    fn start_service(&mut self, _name: &str, _wait: bool) -> Result<(), u32> {
        Ok(())
    }
    fn stop_service(&mut self, _name: &str, _wait: bool) -> Result<(), u32> {
        Ok(())
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
}

/// A UI that answers nothing (/qn), optionally cancelling once progress
/// passes `cancel_at` bytes.
struct Quiet {
    cancel_at: Option<u64>,
    messages: Vec<(u32, String)>,
}
impl Ui for Quiet {
    fn level(&self) -> u32 {
        level::NONE
    }
    fn message(&mut self, kind: u32, text: &str) -> i32 {
        self.messages.push((kind, text.to_string()));
        0
    }
    fn progress(&mut self, done: u64, _total: u64) -> bool {
        self.cancel_at.is_none_or(|c| done < c)
    }
}
fn quiet() -> Quiet {
    Quiet {
        cancel_at: None,
        messages: Vec::new(),
    }
}

fn ada() -> Machine {
    Machine::doors("C:\\Users\\Ada\\", "Ada", "DOORS-PC", true)
}
/// msiexec on `m`: the package (saved in Downloads) and the request.
fn msiexec(
    m: &mut Memory,
    package: &[u8],
    name: &str,
    request: Request,
    props: &[(&str, &str)],
) -> u32 {
    let path = format!("C:\\Users\\Ada\\Downloads\\{name}");
    m.files.insert(path.to_ascii_lowercase(), package.to_vec());
    let db = Database::open(package).unwrap();
    let props: Vec<(String, String)> = props
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut s = Session::new(&db, m, &path, request, ada(), &props, level::NONE);
    s.run(&mut quiet())
}
/// msiexec /x {code}: removal from the cached package.
fn remove(m: &mut Memory, product: &str) -> u32 {
    let Some(cached) = execute::cached_package(product, m) else {
        return code::UNKNOWN_PRODUCT;
    };
    let bytes = m.read_file(&cached).unwrap();
    let db = Database::open(&bytes).unwrap();
    let mut s = Session::new(&db, m, &cached, Request::Remove, ada(), &[], level::NONE);
    s.run(&mut quiet())
}
fn payload() -> fixtures::Payload {
    fixtures::Payload {
        tool: tool_image(1),
        service: None,
        custom_action_dll: None,
    }
}
/// A stand-in PE with a version resource (major `v`).
fn tool_image(v: u16) -> Vec<u8> {
    // DOS header, PE header with one section holding a resource directory
    // whose RT_VERSION leaf has a VS_FIXEDFILEINFO.
    let mut b = vec![0u8; 0x400];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3c] = 0x80;
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    b[0x86] = 1; // one section
    b[0x94] = 0xf0; // optional header size
    b[0x98] = 0x0b;
    b[0x99] = 0x02; // PE32+
    let dirs = 0x98 + 112;
    // Resource directory: RVA 0x1000.
    b[dirs + 16..dirs + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    let section = 0x98 + 0xf0;
    b[section..section + 5].copy_from_slice(b".rsrc");
    b[section + 8..section + 12].copy_from_slice(&0x200u32.to_le_bytes());
    b[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    b[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    b[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
    let r = 0x200;
    // Root: one id entry (16) -> subdirectory at 0x18.
    b[r + 14] = 1;
    b[r + 16..r + 20].copy_from_slice(&16u32.to_le_bytes());
    b[r + 20..r + 24].copy_from_slice(&(0x8000_0018u32).to_le_bytes());
    // Name level -> language level at 0x30.
    let n = r + 0x18;
    b[n + 14] = 1;
    b[n + 16..n + 20].copy_from_slice(&1u32.to_le_bytes());
    b[n + 20..n + 24].copy_from_slice(&(0x8000_0030u32).to_le_bytes());
    let l = r + 0x30;
    b[l + 14] = 1;
    b[l + 16..l + 20].copy_from_slice(&1033u32.to_le_bytes());
    b[l + 20..l + 24].copy_from_slice(&0x48u32.to_le_bytes());
    // Data entry at 0x48: RVA 0x1060, size 0x60.
    let d = r + 0x48;
    b[d..d + 4].copy_from_slice(&0x1060u32.to_le_bytes());
    b[d + 4..d + 8].copy_from_slice(&0x60u32.to_le_bytes());
    let v_at = r + 0x60 + 0x28;
    b[v_at..v_at + 4].copy_from_slice(&0xfeef_04bdu32.to_le_bytes());
    b[v_at + 8..v_at + 12].copy_from_slice(&(u32::from(v) << 16).to_le_bytes());
    b
}

#[test]
fn packed_guids_follow_the_installer() {
    // Windows' packing of {6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}.
    assert_eq!(
        packed_guid("{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}").as_deref(),
        Some("B3C2E1F6D7A8F5E4A9B0C1D2E3F4A5B6")
    );
    assert_eq!(
        unpacked_guid("B3C2E1F6D7A8F5E4A9B0C1D2E3F4A5B6").as_deref(),
        Some("{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}")
    );
    assert_eq!(packed_guid("{not-a-guid}"), None);
}

#[test]
fn links_round_trip() {
    let link = Link {
        target: "C:\\Program Files\\Doors Sample\\readme.txt".to_string(),
        arguments: "/view".to_string(),
        description: "Read me".to_string(),
        working_directory: "C:\\Program Files\\Doors Sample".to_string(),
        show: 1,
    };
    let bytes = link.encode();
    assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0]);
    assert_eq!(Link::decode(&bytes), Some(link));
}

#[test]
fn the_writer_builds_databases_the_reader_opens() {
    let package = fixtures::tools(1, &payload());
    let db = Database::open(&package).unwrap();
    assert_eq!(db.property("ProductName").as_deref(), Some("Doors Tools"));
    assert_eq!(
        db.property("ProductCode").as_deref(),
        Some(fixtures::TOOLS_1)
    );
    let file = db.table("File").unwrap();
    assert_eq!(file.rows.len(), 4);
    let tool = file.find("fil_tool").unwrap();
    assert_eq!(
        file.get(tool, "FileName").str(),
        Some("TOOLPR~1.EXE|toolprobe.exe")
    );
    assert_eq!(file.get(tool, "Version").str(), Some("1.0.0.0"));
    assert_eq!(file.get(tool, "Attributes").int(), Some(0x4000));
    assert!(file.columns[0].is_key());
    // Integers of both widths, nulls, the catalog's types.
    let registry = db.table("Registry").unwrap();
    let runs = registry.find("reg_runs").unwrap();
    assert_eq!(registry.get(runs, "Root").int(), Some(2));
    assert!(
        registry
            .get(registry.find("reg_key").unwrap(), "Value")
            .is_null()
    );
    let columns: Vec<u16> = db
        .tables
        .iter()
        .find(|t| t.0 == "File")
        .unwrap()
        .1
        .iter()
        .map(|c| c.kind)
        .collect();
    assert_eq!(
        columns,
        [
            0x2d48, 0x0d48, 0x0fff, 0x0104, 0x1d48, 0x1d14, 0x1502, 0x0104
        ]
    );
    let sequence = db.table("InstallExecuteSequence").unwrap();
    let finalize = sequence.find("InstallFinalize").unwrap();
    assert_eq!(sequence.get(finalize, "Sequence").int(), Some(6600));
    // The cabinet and the summary.
    let cab = db.stream("tools.cab").unwrap();
    let cab = door_cab::Cabinet::parse(&cab).unwrap();
    assert_eq!(
        cab.extract(cab.file("fil_readme").unwrap()).unwrap(),
        b"Doors Tools 1.0\r\nThe first release.\r\n"
    );
    let s = db.summary().unwrap();
    assert_eq!(s.text(summary::pid::SUBJECT), Some("Doors Tools"));
    assert_eq!(s.platform(), Some("x64"));
    assert!(s.compressed());
    assert_eq!(s.int(summary::pid::PAGE_COUNT), Some(500));
    // A large stream goes past the mini stream; many tables, many strings.
    let mut b = write::Builder::new();
    b.table(
        "Property",
        &[
            ("Property", write::kind::key(write::kind::string(72))),
            ("Value", write::kind::text(0)),
        ],
    );
    for i in 0..3000 {
        b.row_text("Property", &[&format!("P{i:04}"), &format!("value {i}")]);
    }
    b.stream("Big", vec![7u8; 300_000]);
    let bytes = b.build();
    let db = Database::open(&bytes).unwrap();
    assert_eq!(db.property("P2999").as_deref(), Some("value 2999"));
    assert_eq!(db.stream("Big").unwrap(), vec![7u8; 300_000]);
}

#[test]
fn installs_and_removes_the_windows_built_sample() {
    let mut m = Memory::machine();
    assert_eq!(
        msiexec(&mut m, SAMPLE, "sample.msi", Request::Install, &[]),
        0
    );
    assert_eq!(
        m.file("C:\\Program Files\\Doors Sample\\readme.txt")
            .map(Vec::as_slice),
        Some(&b"Doors sample product\r\n"[..])
    );
    let app = m
        .file("C:\\Program Files\\Doors Sample\\data\\app.bin")
        .unwrap();
    assert_eq!(app.len(), 50000);
    assert!(
        app.iter()
            .enumerate()
            .all(|(i, b)| *b == ((i * 31) % 251) as u8)
    );
    let lnk = m
        .file("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Doors Sample readme.lnk")
        .unwrap();
    assert_eq!(
        Link::decode(lnk).unwrap().target,
        "C:\\Program Files\\Doors Sample\\readme.txt"
    );
    let sample = "Software\\Macrohard Test\\Doors Sample";
    assert_eq!(
        m.get(Root::LocalMachine, sample, "Runs"),
        Some(RegValue::Dword(42))
    );
    let arp = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}";
    assert_eq!(
        m.get(Root::LocalMachine, arp, "DisplayName"),
        Some(RegValue::String("Doors Sample".to_string()))
    );
    assert_eq!(
        m.get(Root::LocalMachine, arp, "InstallDate"),
        Some(RegValue::String("20261108".to_string()))
    );
    let cached = execute::cached_package("{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}", &mut m).unwrap();
    assert!(cached.starts_with("C:\\Windows\\Installer\\") && cached.ends_with(".msi"));
    assert_eq!(m.file(&cached).map(Vec::len), Some(SAMPLE.len()));
    // Committed: no rollback script, no backups, nothing in progress.
    assert!(
        !m.files
            .keys()
            .any(|f| f.starts_with("c:\\windows\\installer\\config.msi\\"))
    );
    assert!(!m.folders.iter().any(|f| f.ends_with("config.msi")));
    assert!(
        m.values(Root::LocalMachine, journal::IN_PROGRESS)
            .is_empty()
    );
    assert!(m.log.iter().any(|l| l == "Action start: InstallFiles."));

    // The sample's InstallExecuteSequence has no RemoveFiles,
    // RemoveShortcuts or RemoveRegistryValues: as on Windows, its removal
    // unregisters the product and leaves what those actions would take.
    assert_eq!(remove(&mut m, "{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}"), 0);
    assert!(
        m.file("C:\\Program Files\\Doors Sample\\readme.txt")
            .is_some()
    );
    assert_eq!(
        m.get(Root::LocalMachine, sample, "Runs"),
        Some(RegValue::Dword(42))
    );
    assert!(
        m.files
            .contains_key("c:\\users\\ada\\downloads\\sample.msi")
    );
    assert_eq!(m.get(Root::LocalMachine, arp, "DisplayName"), None);
    assert_eq!(
        execute::cached_package("{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}", &mut m),
        None
    );
    assert!(
        !m.files
            .keys()
            .any(|f| f.starts_with("c:\\windows\\installer\\"))
    );
    assert_eq!(
        remove(&mut m, "{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}"),
        code::UNKNOWN_PRODUCT
    );
}

/// A DLL custom action's property reaches the registry: WriteRegistryValues
/// formats values after the action ran, not at costing.
#[test]
fn a_dll_custom_action_property_reaches_the_registry() {
    let mut m = Memory::machine();
    let package = fixtures::tools(
        1,
        &fixtures::Payload {
            custom_action_dll: Some(b"MZ caprobe stand-in".to_vec()),
            ..payload()
        },
    );
    assert_eq!(
        msiexec(&mut m, &package, "tools-1.0.msi", Request::Install, &[]),
        0,
        "{:#?}",
        m.log
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            "Software\\Macrohard Test\\Doors Tools",
            "CaCheck"
        ),
        Some(RegValue::String(String::from("passed")))
    );
    // The extracted DLL is gone again.
    assert!(!m.files.keys().any(|f| f.ends_with(".tmp")));
}
/// The MSVC-built probe INSTALL-A ships, when `DOORS_TOOLPROBE` names it:
/// the cabinet carries it through InstallFiles byte for byte.
#[test]
fn installs_the_real_probe_byte_for_byte() {
    let Ok(path) = std::env::var("DOORS_TOOLPROBE") else {
        return;
    };
    let tool = std::fs::read(path).unwrap();
    let mut m = Memory::machine();
    let package = fixtures::tools(
        1,
        &fixtures::Payload {
            tool: tool.clone(),
            ..payload()
        },
    );
    assert_eq!(
        msiexec(&mut m, &package, "tools-1.0.msi", Request::Install, &[]),
        0,
        "{:#?}",
        m.log
    );
    let installed = m
        .file("C:\\Program Files\\Doors Tools\\toolprobe.exe")
        .unwrap();
    assert_eq!(installed.len(), tool.len());
    assert!(*installed == tool, "the installed program differs");
}
#[test]
fn installs_doors_tools_with_its_registrations() {
    let mut m = Memory::machine();
    let package = fixtures::tools(1, &payload());
    assert_eq!(
        msiexec(&mut m, &package, "tools-1.0.msi", Request::Install, &[]),
        0,
        "{:#?}",
        m.log
    );
    let folder = "C:\\Program Files\\Doors Tools\\";
    assert_eq!(
        m.file(&format!("{folder}toolprobe.exe")),
        Some(&tool_image(1))
    );
    assert!(
        m.file("C:\\Program Files\\Common Files\\Doors\\doors-shared.txt")
            .is_some()
    );
    // The registry, the file association, the shortcut.
    let tools = "Software\\Macrohard Test\\Doors Tools";
    assert_eq!(
        m.get(Root::LocalMachine, tools, "Version"),
        Some(RegValue::String("1.0.0".to_string()))
    );
    assert_eq!(
        m.get(Root::LocalMachine, tools, "InstallPath"),
        Some(RegValue::String(folder.to_string()))
    );
    assert_eq!(
        m.get(Root::LocalMachine, "Software\\Classes\\.doorsnote", ""),
        Some(RegValue::String("DoorsTools.Note".to_string()))
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            "Software\\Classes\\DoorsTools.Note\\shell\\open\\command",
            ""
        ),
        Some(RegValue::String(format!(
            "\"{folder}toolprobe.exe\" open \"%1\""
        )))
    );
    let lnk = m
        .file("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Doors Tools.lnk")
        .unwrap();
    let link = Link::decode(lnk).unwrap();
    assert_eq!(link.target, format!("{folder}toolprobe.exe"));
    assert_eq!(link.working_directory, folder);
    // The Installer's records: product, features, upgrade code, components.
    let record = records::product(&mut m, fixtures::TOOLS_1).unwrap();
    assert_eq!(record.name, "Doors Tools");
    assert_eq!(record.version, "1.0.0");
    assert_eq!(record.features, ["Main"]);
    assert!(
        record
            .properties
            .contains(&("INSTALLFOLDER".to_string(), folder.to_string()))
    );
    assert_eq!(
        records::related(&mut m, fixtures::TOOLS_UPGRADE),
        [fixtures::TOOLS_1]
    );
    let clients = records::clients(&mut m, fixtures::SHARED_COMPONENT);
    assert_eq!(clients.len(), 1);
    assert_eq!(
        clients[0].1,
        "C:\\Program Files\\Common Files\\Doors\\doors-shared.txt"
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            records::SHARED_DLLS,
            &records::shared_dll_name("C:\\Program Files\\Common Files\\Doors\\doors-shared.txt")
        ),
        Some(RegValue::Dword(1))
    );
    // The deferred EXE custom action ran the installed program.
    assert!(
        m.ran.iter().any(
            |r| r.ends_with("toolprobe.exe\" mark \"C:\\ProgramData\\Doors Tools Marker.txt\"")
        ),
        "{:?}",
        m.ran
    );
    assert!(m.file("C:\\ProgramData\\Doors Tools Marker.txt").is_some());
    assert_eq!(records::products(&mut m), [fixtures::TOOLS_1]);
    // A second /i of the same package: maintenance, nothing changes.
    assert_eq!(
        msiexec(&mut m, &package, "tools-1.0.msi", Request::Install, &[]),
        0
    );
}

#[test]
fn shared_components_stay_while_another_product_uses_them() {
    let mut m = Memory::machine();
    let shared = "C:\\Program Files\\Common Files\\Doors\\doors-shared.txt";
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::tools(1, &payload()),
            "tools.msi",
            Request::Install,
            &[]
        ),
        0
    );
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::companion(),
            "companion.msi",
            Request::Install,
            &[]
        ),
        0
    );
    assert_eq!(
        records::clients(&mut m, fixtures::SHARED_COMPONENT).len(),
        2
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            records::SHARED_DLLS,
            &records::shared_dll_name(shared)
        ),
        Some(RegValue::Dword(2))
    );
    // The user keeps a file of their own in the tools folder.
    m.write_file("C:\\Program Files\\Doors Tools\\my notes.txt", b"mine")
        .unwrap();
    m.write_file("C:\\Program Files\\Doors Tools\\debug.log", b"log")
        .unwrap();
    assert_eq!(remove(&mut m, fixtures::TOOLS_1), 0, "{:#?}", m.log);
    assert!(m.file(shared).is_some(), "the companion still uses it");
    assert_eq!(
        m.get(
            Root::LocalMachine,
            records::SHARED_DLLS,
            &records::shared_dll_name(shared)
        ),
        Some(RegValue::Dword(1))
    );
    assert_eq!(
        records::clients(&mut m, fixtures::SHARED_COMPONENT).len(),
        1
    );
    assert!(
        m.file("C:\\Program Files\\Doors Tools\\toolprobe.exe")
            .is_none()
    );
    // The package's RemoveFile took the log; the user's notes stay, and so
    // does their folder.
    assert!(
        m.file("C:\\Program Files\\Doors Tools\\debug.log")
            .is_none()
    );
    assert_eq!(
        m.file("C:\\Program Files\\Doors Tools\\my notes.txt")
            .map(Vec::as_slice),
        Some(&b"mine"[..])
    );
    // The registry key the package removes goes; unrelated keys stay.
    assert!(
        m.values(Root::LocalMachine, "Software\\Macrohard Test\\Doors Tools")
            .is_empty()
    );
    assert_eq!(
        m.get(Root::LocalMachine, "Software\\Classes\\.doorsnote", ""),
        None
    );
    assert_eq!(remove(&mut m, fixtures::COMPANION), 0);
    assert!(m.file(shared).is_none());
    assert_eq!(
        m.get(
            Root::LocalMachine,
            records::SHARED_DLLS,
            &records::shared_dll_name(shared)
        ),
        None
    );
    assert!(records::clients(&mut m, fixtures::SHARED_COMPONENT).is_empty());
    assert!(records::products(&mut m).is_empty());
}

#[test]
fn a_major_upgrade_replaces_the_old_version_and_keeps_the_users_changes() {
    let mut m = Memory::machine();
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::tools(1, &payload()),
            "tools-1.0.msi",
            Request::Install,
            &[]
        ),
        0
    );
    // The user edits the settings and adds a file.
    let settings = "C:\\Program Files\\Doors Tools\\settings.ini";
    m.write_file(settings, b"[Tools]\r\nTheme=Dark\r\n")
        .unwrap();
    m.write_file("C:\\Program Files\\Doors Tools\\my notes.txt", b"mine")
        .unwrap();
    let v2 = fixtures::tools(
        2,
        &fixtures::Payload {
            tool: tool_image(2),
            ..payload()
        },
    );
    assert_eq!(
        msiexec(&mut m, &v2, "tools-2.0.msi", Request::Install, &[]),
        0,
        "{:#?}",
        m.log
    );
    assert!(
        m.log
            .iter()
            .any(|l| l.contains("RemoveExistingProducts: removing {3D1C9A57-0001")),
        "{:#?}",
        m.log
    );
    assert!(records::product(&mut m, fixtures::TOOLS_1).is_none());
    let record = records::product(&mut m, fixtures::TOOLS_2).unwrap();
    assert_eq!(record.version, "2.0.0");
    assert_eq!(
        records::related(&mut m, fixtures::TOOLS_UPGRADE),
        [fixtures::TOOLS_2]
    );
    assert_eq!(
        m.file("C:\\Program Files\\Doors Tools\\toolprobe.exe"),
        Some(&tool_image(2))
    );
    assert_eq!(
        m.file("C:\\Program Files\\Doors Tools\\readme.txt")
            .map(Vec::as_slice),
        Some(&b"Doors Tools 2.0\r\nThe second release.\r\n"[..])
    );
    // The removal of 1.0 took its settings.ini (it owned it); 2.0 put its
    // own back. The user's notes are untouched.
    assert_eq!(
        m.file("C:\\Program Files\\Doors Tools\\my notes.txt")
            .map(Vec::as_slice),
        Some(&b"mine"[..])
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            "Software\\Macrohard Test\\Doors Tools",
            "Version"
        ),
        Some(RegValue::String("2.0.0".to_string()))
    );
    // 1.0 cannot go over 2.0.
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::tools(1, &payload()),
            "tools-1.0.msi",
            Request::Install,
            &[]
        ),
        code::INSTALL_FAILURE
    );
    assert_eq!(
        records::product(&mut m, fixtures::TOOLS_2).unwrap().version,
        "2.0.0"
    );
}

#[test]
fn a_repair_keeps_a_users_edit_of_an_unversioned_file_and_restores_missing_ones() {
    let mut m = Memory::machine();
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::tools(1, &payload()),
            "tools.msi",
            Request::Install,
            &[]
        ),
        0
    );
    let settings = "C:\\Program Files\\Doors Tools\\settings.ini";
    m.write_file(settings, b"[Tools]\r\nTheme=Dark\r\n")
        .unwrap();
    m.remove_file("C:\\Program Files\\Doors Tools\\toolprobe.exe")
        .unwrap();
    m.delete_value(
        Root::LocalMachine,
        "Software\\Macrohard Test\\Doors Tools",
        Some("Runs"),
    );
    let cached = execute::cached_package(fixtures::TOOLS_1, &mut m).unwrap();
    let bytes = m.read_file(&cached).unwrap();
    let db = Database::open(&bytes).unwrap();
    let mut s = Session::new(
        &db,
        &mut m,
        &cached,
        Request::Repair("omus".into()),
        ada(),
        &[],
        level::NONE,
    );
    assert_eq!(s.run(&mut quiet()), 0);
    assert_eq!(
        m.file("C:\\Program Files\\Doors Tools\\toolprobe.exe"),
        Some(&tool_image(1))
    );
    assert_eq!(
        m.get(
            Root::LocalMachine,
            "Software\\Macrohard Test\\Doors Tools",
            "Runs"
        ),
        Some(RegValue::Dword(42))
    );
    assert_eq!(
        m.file(settings).map(Vec::as_slice),
        Some(&b"[Tools]\r\nTheme=Dark\r\n"[..])
    );
    assert!(m.log.iter().any(|l| l.contains("unversioned but modified")));
    // A repair of something not installed.
    let mut fresh = Memory::machine();
    let db = Database::open(&bytes).unwrap();
    let mut s = Session::new(
        &db,
        &mut fresh,
        &cached,
        Request::Repair("omus".into()),
        ada(),
        &[],
        level::NONE,
    );
    assert_eq!(s.run(&mut quiet()), code::UNKNOWN_PRODUCT);
}

/// The machine is exactly as before: nothing installed, nothing left over.
fn assert_clean(m: &Memory, before: &Memory) {
    let files: Vec<&String> = m
        .files
        .keys()
        .filter(|f| !before.files.contains_key(*f))
        .collect();
    assert!(files.is_empty(), "files left: {files:?}");
    let folders: Vec<&String> = m
        .folders
        .iter()
        .filter(|f| !before.folders.contains(f))
        .collect();
    assert!(folders.is_empty(), "folders left: {folders:?}");
    let values: Vec<_> = m
        .registry
        .keys()
        .filter(|k| !before.registry.contains_key(*k))
        .collect();
    assert!(values.is_empty(), "registry left: {values:?}");
}

#[test]
fn a_failure_rolls_everything_back_including_the_custom_actions_work() {
    let mut m = Memory::machine();
    let mut before = m.clone();
    before.files.insert(
        "c:\\users\\ada\\downloads\\tools.msi".to_string(),
        Vec::new(),
    );
    let status = msiexec(
        &mut m,
        &fixtures::tools(1, &payload()),
        "tools.msi",
        Request::Install,
        &[("FAIL_AT", "files")],
    );
    assert_eq!(status, code::INSTALL_FAILURE);
    assert_clean(&m, &before);
    // The marker the deferred action wrote was removed by its rollback
    // action; the log keeps the record of the nontransactional action.
    assert!(m.ran.iter().any(|r| r.contains(" unmark ")), "{:?}", m.ran);
    assert!(
        m.log
            .iter()
            .any(|l| l.contains("Rollback could not undo custom action RecordInstall"))
    );
    assert!(
        m.values(Root::LocalMachine, journal::IN_PROGRESS)
            .is_empty()
    );
}

#[test]
fn cancel_rolls_back_with_1602() {
    let mut m = Memory::machine();
    let mut before = m.clone();
    before.files.insert(
        "c:\\users\\ada\\downloads\\tools.msi".to_string(),
        Vec::new(),
    );
    let package = fixtures::tools(1, &payload());
    let path = "C:\\Users\\Ada\\Downloads\\tools.msi";
    m.files.insert(path.to_ascii_lowercase(), package.clone());
    let db = Database::open(&package).unwrap();
    let mut s = Session::new(&db, &mut m, path, Request::Install, ada(), &[], level::NONE);
    let mut ui = Quiet {
        cancel_at: Some(100),
        messages: Vec::new(),
    };
    assert_eq!(s.run(&mut ui), code::INSTALL_USEREXIT);
    assert_clean(&m, &before);
}

#[test]
fn an_interrupted_installation_is_rolled_back_by_the_next_session() {
    let mut m = Memory::machine();
    let package = fixtures::tools(1, &payload());
    let mut before = m.clone();
    before.files.insert(
        "c:\\users\\ada\\downloads\\tools.msi".to_string(),
        Vec::new(),
    );
    msiexec(
        &mut m,
        &package,
        "tools.msi",
        Request::Install,
        &[("HANG", "1")],
    );
    // The machine at the moment the program hung: files copied, the
    // marker written, the script in progress.
    let mut crashed = *m.crash.take().expect("the hang ran");
    assert!(
        crashed
            .file("C:\\Program Files\\Doors Tools\\toolprobe.exe")
            .is_some()
    );
    assert!(
        crashed
            .file("C:\\ProgramData\\Doors Tools Marker.txt")
            .is_some()
    );
    assert!(
        !crashed
            .values(Root::LocalMachine, journal::IN_PROGRESS)
            .is_empty()
    );
    // Same boot: another installation is running.
    assert_eq!(
        msiexec(
            &mut crashed.clone(),
            &fixtures::companion(),
            "companion.msi",
            Request::Install,
            &[]
        ),
        code::INSTALL_ALREADY_RUNNING
    );
    // The next boot: the next session rolls it back, then installs.
    crashed.boot = 1;
    assert_eq!(
        msiexec(
            &mut crashed,
            &fixtures::companion(),
            "companion.msi",
            Request::Install,
            &[]
        ),
        0
    );
    assert!(
        crashed
            .log
            .iter()
            .any(|l| l.starts_with("DOOR_P16 MSI ROLLBACK interrupted product={3D1C9A57-0001")),
        "{:#?}",
        crashed.log
    );
    assert!(
        crashed
            .file("C:\\Program Files\\Doors Tools\\toolprobe.exe")
            .is_none()
    );
    assert!(records::product(&mut crashed, fixtures::TOOLS_1).is_none());
    // The marker: its rollback action ran from the script's recorded
    // command line.
    assert!(
        crashed.ran.iter().any(|r| r.contains(" unmark ")),
        "{:?}",
        crashed.ran
    );
    assert!(
        crashed
            .file("C:\\ProgramData\\Doors Tools Marker.txt")
            .is_none()
    );
    assert!(records::product(&mut crashed, fixtures::COMPANION).is_some());
    let leftovers: Vec<&String> = crashed
        .files
        .keys()
        .filter(|f| f.contains("config.msi") || f.contains("doors tools"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn not_enough_disk_space_stops_before_anything_is_copied() {
    let mut m = Memory::machine();
    m.free = Some(1 << 30);
    let mut before = m.clone();
    before
        .files
        .insert("c:\\users\\ada\\downloads\\big.msi".to_string(), Vec::new());
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::oversized(),
            "big.msi",
            Request::Install,
            &[]
        ),
        code::INSTALL_FAILURE
    );
    assert!(m.log.iter().any(|l| l.starts_with("Out of disk space -- Volume: 'C:'; required space: 1855469 KB; available space: 1048576 KB.")), "{:#?}", m.log);
    assert_clean(&m, &before);
}

#[test]
fn product_codes_and_reboots_follow_windows_installer() {
    let mut m = Memory::machine();
    assert_eq!(
        msiexec(
            &mut m,
            &fixtures::tools(1, &payload()),
            "tools.msi",
            Request::Install,
            &[("NEEDREBOOT", "1")]
        ),
        code::SUCCESS_REBOOT_REQUIRED
    );
    // The same product code from another package: another version of the
    // product is installed.
    let mut other = fixtures::tools(1, &payload());
    let db = Database::open(&other).unwrap();
    let _ = db;
    // Rebuild with a different package code by patching the summary's
    // revision GUID text.
    let needle = b"{7B0E4D22-0001-4C1A-B8F3-5D6E7A8B9C01}";
    let at = other
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap();
    other[at + 1] = b'8';
    assert_eq!(
        msiexec(&mut m, &other, "tools-other.msi", Request::Install, &[]),
        code::PRODUCT_VERSION
    );
    // Per-machine without the token: 1925 quietly.
    struct Standard(Memory);
    let mut user = Memory::machine();
    let package = fixtures::companion();
    let path = "C:\\Users\\Ada\\Downloads\\companion.msi";
    user.files
        .insert(path.to_ascii_lowercase(), package.clone());
    let db = Database::open(&package).unwrap();
    let mut standard = Standard(user);
    impl Target for Standard {
        fn create_folder(&mut self, p: &str) -> Result<(), u32> {
            self.0.create_folder(p)
        }
        fn write_file(&mut self, p: &str, d: &[u8]) -> Result<(), u32> {
            self.0.write_file(p, d)
        }
        fn read_file(&mut self, p: &str) -> Result<Vec<u8>, u32> {
            self.0.read_file(p)
        }
        fn exists(&mut self, p: &str) -> bool {
            self.0.exists(p)
        }
        fn remove_file(&mut self, p: &str) -> Result<(), u32> {
            self.0.remove_file(p)
        }
        fn remove_folder(&mut self, p: &str) -> Result<(), u32> {
            self.0.remove_folder(p)
        }
        fn set_value(
            &mut self,
            r: Root,
            k: &str,
            n: Option<&str>,
            v: &RegValue,
        ) -> Result<(), u32> {
            self.0.set_value(r, k, n, v)
        }
        fn create_key(&mut self, r: Root, k: &str) -> Result<(), u32> {
            self.0.create_key(r, k)
        }
        fn value(&mut self, r: Root, k: &str, n: &str) -> Option<RegValue> {
            self.0.value(r, k, n)
        }
        fn delete_value(&mut self, r: Root, k: &str, n: Option<&str>) {
            self.0.delete_value(r, k, n)
        }
        fn delete_key(&mut self, r: Root, k: &str) {
            self.0.delete_key(r, k)
        }
        fn elevated(&mut self) -> bool {
            false
        }
    }
    let mut s = Session::new(
        &db,
        &mut standard,
        path,
        Request::Install,
        ada(),
        &[],
        level::NONE,
    );
    assert_eq!(s.run(&mut quiet()), code::INSUFFICIENT_PRIVILEGES);
    // With a UI the session asks for elevation instead.
    struct Basic;
    impl Ui for Basic {
        fn level(&self) -> u32 {
            level::BASIC
        }
        fn message(&mut self, _kind: u32, _text: &str) -> i32 {
            0
        }
    }
    let mut s = Session::new(
        &db,
        &mut standard,
        path,
        Request::Install,
        ada(),
        &[],
        level::BASIC,
    );
    assert_eq!(s.run(&mut Basic), code::ELEVATION_REQUIRED);
    let _ = msg::IDOK;
}

#[test]
fn msiexec_command_lines_parse_as_windows_does() {
    use engine::{parse_command_line, split_msiexec_line};
    let p = |s: &str| parse_command_line(&split_msiexec_line(s));
    let a = p("/i \"C:\\Users\\Ada\\Doors Tools.msi\" /qn /l*v install.log INSTALLFOLDER=\"D:\\Apps\\Tools\" /norestart").unwrap();
    assert_eq!(
        a.install.as_deref(),
        Some("C:\\Users\\Ada\\Doors Tools.msi")
    );
    assert_eq!(a.ui, level::NONE);
    assert_eq!(a.log.as_deref(), Some("install.log"));
    assert!(
        a.properties
            .contains(&("INSTALLFOLDER".to_string(), "D:\\Apps\\Tools".to_string()))
    );
    assert!(
        a.properties
            .contains(&("REBOOT".to_string(), "ReallySuppress".to_string()))
    );
    let x =
        p("MsiExec.exe /X{3D1C9A57-0001-4F60-9C14-7A5B2E0D6F11}"
            .trim_start_matches("MsiExec.exe "))
        .unwrap();
    assert_eq!(x.remove.as_deref(), Some(fixtures::TOOLS_1));
    assert_eq!(x.ui, level::FULL);
    let f = p("/fomus {3D1C9A57-0001-4F60-9C14-7A5B2E0D6F11} /passive").unwrap();
    assert_eq!(
        f.repair,
        Some(("omus".to_string(), fixtures::TOOLS_1.to_string()))
    );
    assert!(f.passive);
    assert_eq!(f.ui, level::BASIC);
    assert_eq!(p("/f {X}").unwrap().repair.unwrap().0, "pecms");
    assert_eq!(p("/i a.msi /x b.msi"), Err(code::INVALID_COMMAND_LINE));
    assert_eq!(p("/z a.msi"), Err(code::INVALID_COMMAND_LINE));
    assert_eq!(p(""), Err(code::INVALID_COMMAND_LINE));
    assert!(p("/?").unwrap().help);
    assert_eq!(p("/qb /i a.msi").unwrap().ui, level::BASIC);
    // msiexec's own quoting: a folder's trailing backslash before the
    // closing quote stays a backslash; "" inside quotes is a quote.
    let h = p(
        "/i a.msi INSTALLFOLDER=\"C:\\Program Files\\Doors Tools\\\" NOTE=\"say \"\"hi\"\"\" /qb",
    )
    .unwrap();
    assert!(h.properties.contains(&(
        "INSTALLFOLDER".to_string(),
        "C:\\Program Files\\Doors Tools\\".to_string()
    )));
    assert!(
        h.properties
            .contains(&("NOTE".to_string(), "say \"hi\"".to_string()))
    );
    assert_eq!(h.ui, level::BASIC);
}

#[test]
fn rollback_scripts_round_trip() {
    use journal::Undo;
    for u in [
        Undo::Created("C:\\a\tb.txt".to_string()),
        Undo::Replaced {
            path: "C:\\x".to_string(),
            backup: "C:\\Config.Msi\\1-1.rbf".to_string(),
        },
        Undo::Value {
            root: Root::LocalMachine,
            key: "Software\\K".to_string(),
            name: "".to_string(),
            previous: Some(RegValue::Multi(vec!["a".to_string(), "b\\c".to_string()])),
        },
        Undo::Value {
            root: Root::CurrentUser,
            key: "K".to_string(),
            name: "n".to_string(),
            previous: None,
        },
        Undo::Value {
            root: Root::LocalMachine,
            key: "K".to_string(),
            name: "b".to_string(),
            previous: Some(RegValue::Binary(vec![0, 255, 16])),
        },
        Undo::Action {
            name: "Rb".to_string(),
            kind: 0x522,
            source: "C:\\".to_string(),
            target: "\"a.exe\" x".to_string(),
            data: "line\nbreak".to_string(),
        },
        Undo::Nontransactional("RecordInstall".to_string()),
    ] {
        let line = u.line();
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(Undo::parse(line.trim_end_matches('\n')), Some(u));
    }
}

/// Interop: writes the fixture packages to `DOOR_MSI_OUT` for Windows
/// Installer's engine to open read-only (run with `--ignored`).
#[test]
#[ignore]
fn write_packages_for_windows() {
    let Some(dir) = std::env::var_os("DOOR_MSI_OUT") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::write(dir.join("tools-1.0.msi"), fixtures::tools(1, &payload())).unwrap();
    std::fs::write(dir.join("tools-2.0.msi"), fixtures::tools(2, &payload())).unwrap();
    std::fs::write(dir.join("companion.msi"), fixtures::companion()).unwrap();
}

#[test]
fn registers_classes_fonts_environment_and_ini_files_and_removes_them() {
    const ENVIRONMENT: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";
    let mut m = Memory::machine();
    // The system Path the package appends to.
    m.set_value(
        Root::LocalMachine,
        ENVIRONMENT,
        Some("Path"),
        &RegValue::Expand("%SystemRoot%;C:\\Tools".to_string()),
    )
    .unwrap();
    let package = fixtures::registrations();
    assert_eq!(
        msiexec(&mut m, &package, "registrations.msi", Request::Install, &[]),
        0,
        "{:#?}",
        m.log
    );
    let folder = "C:\\Program Files (x86)\\Doors Registrations\\";
    let s = |v: &str| Some(RegValue::String(v.to_string()));
    // A 32-bit package's COM class, ProgIds, AppId and MIME type: in the
    // 32-bit view.
    let clsid = format!("Software\\Classes\\CLSID\\{}", fixtures::REG_CLSID);
    let view = Root::LocalMachine32;
    assert_eq!(m.get(view, &clsid, ""), s("Doors Registrations Server"));
    assert_eq!(
        m.get(view, &format!("{clsid}\\LocalServer32"), ""),
        s(&format!("{folder}server.exe /automation"))
    );
    assert_eq!(
        m.get(view, &format!("{clsid}\\ProgID"), ""),
        s("Doors.Server.1")
    );
    assert_eq!(
        m.get(view, &format!("{clsid}\\VersionIndependentProgID"), ""),
        s("Doors.Server")
    );
    assert_eq!(m.get(view, &clsid, "AppID"), s(fixtures::REG_APPID));
    assert_eq!(m.get(Root::LocalMachine, &clsid, ""), None);
    let appid = format!("Software\\Classes\\AppID\\{}", fixtures::REG_APPID);
    assert_eq!(m.get(view, &appid, "ActivateAtStorage"), s("Y"));
    assert_eq!(
        m.get(view, "Software\\Classes\\Doors.Server.1\\CLSID", ""),
        s(fixtures::REG_CLSID)
    );
    assert_eq!(
        m.get(view, "Software\\Classes\\Doors.Server\\CurVer", ""),
        s("Doors.Server.1")
    );
    let mime = "Software\\Classes\\MIME\\Database\\Content Type\\application/x-doors-reg";
    assert_eq!(m.get(view, mime, "Extension"), s(".doorsreg"));
    assert_eq!(m.get(view, mime, "CLSID"), s(fixtures::REG_CLSID));
    // Its Add/Remove Programs entry is in the 32-bit view too; the
    // Installer's own records are not.
    let arp = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        fixtures::REGISTRATIONS
    );
    assert_eq!(m.get(view, &arp, "DisplayName"), s("Doors Registrations"));
    assert!(records::product(&mut m, fixtures::REGISTRATIONS).is_some());
    // The font, under the title its file gives.
    let fonts = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
    let title = format!("{} (TrueType)", fixtures::REG_FONT);
    assert_eq!(m.get(Root::LocalMachine, fonts, &title), s("doorsreg.ttf"));
    assert!(m.file("C:\\Windows\\Fonts\\doorsreg.ttf").is_some());
    // The environment: a variable set, a folder appended to Path once.
    assert_eq!(
        m.get(Root::LocalMachine, ENVIRONMENT, "DOORSREG_HOME"),
        s(folder)
    );
    assert_eq!(
        m.get(Root::LocalMachine, ENVIRONMENT, "Path"),
        Some(RegValue::Expand(format!("%SystemRoot%;C:\\Tools;{folder}")))
    );
    // The INI file and the duplicated file.
    let ini = format!("{folder}doorsreg.ini");
    assert_eq!(
        m.file(&ini).map(|b| b.as_slice()),
        Some(format!("[Settings]\r\nHome={folder}\r\nPlugins=core\r\n").as_bytes())
    );
    assert_eq!(
        m.file(&format!("{folder}Data\\readme-copy.txt")),
        m.file(&format!("{folder}readme.txt"))
    );
    // The qualified component: the descriptor and AppData.
    let category = format!(
        "Software\\Classes\\Installer\\Components\\{}",
        packed_guid(fixtures::REG_CATEGORY).unwrap()
    );
    let descriptor = format!(
        "{}Main>{}Doors data",
        install::base85_guid(fixtures::REGISTRATIONS).unwrap(),
        install::base85_guid("{D0025A00-1001-4C11-8A00-5E7A1B2C3D01}").unwrap()
    );
    assert_eq!(
        m.get(Root::LocalMachine, &category, "en-us"),
        Some(RegValue::Multi(vec![descriptor]))
    );

    // Removal takes every one of them back.
    assert_eq!(remove(&mut m, fixtures::REGISTRATIONS), 0, "{:#?}", m.log);
    assert_eq!(m.get(view, &clsid, ""), None);
    assert_eq!(m.get(view, &format!("{clsid}\\LocalServer32"), ""), None);
    assert_eq!(m.get(view, &appid, "ActivateAtStorage"), None);
    assert_eq!(
        m.get(view, "Software\\Classes\\Doors.Server\\CurVer", ""),
        None
    );
    assert_eq!(m.get(view, mime, "Extension"), None);
    assert_eq!(m.get(view, &arp, "DisplayName"), None);
    assert_eq!(m.get(Root::LocalMachine, fonts, &title), None);
    assert_eq!(
        m.get(Root::LocalMachine, ENVIRONMENT, "DOORSREG_HOME"),
        None
    );
    assert_eq!(
        m.get(Root::LocalMachine, ENVIRONMENT, "Path"),
        Some(RegValue::Expand("%SystemRoot%;C:\\Tools".to_string()))
    );
    assert_eq!(m.file(&format!("{folder}Data\\readme-copy.txt")), None);
    assert_eq!(m.get(Root::LocalMachine, &category, "en-us"), None);
    // The INI lines go; the (now empty) section stays, as Windows leaves it.
    assert_eq!(
        m.file(&ini).map(|b| b.as_slice()),
        Some(b"[Settings]\r\n".as_slice())
    );
}

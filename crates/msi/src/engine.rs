//! Windows Installer's engine: a session over a package runs the
//! InstallUISequence (with the package's dialogs, for the full UI) and the
//! InstallExecuteSequence, action by action in sequence order with their
//! conditions. Standard actions (costing, InstallFiles with Windows
//! Installer's file versioning rules, the registry, shortcuts, file
//! associations, services, component registration with its client counts,
//! RegisterProduct/PublishProduct, FindRelatedProducts and
//! RemoveExistingProducts for upgrades) and custom actions (DLL, EXE,
//! property and directory, error; immediate, deferred, rollback and
//! commit) run in place. Every change between InstallInitialize and
//! InstallFinalize is journaled first (`journal`): a failure or Cancel
//! rolls it back, InstallFinalize commits it, and an installation
//! interrupted by a crash is rolled back by the next session. Removal,
//! repair (REINSTALL/REINSTALLMODE) and feature changes work from the
//! installed product's record and cached package.
use crate::execute::{Target, code, packed_guid, records, split_root};
use crate::install::{
    self, Cabinet, ComponentInfo, Machine, Plan, RegValue, RegistryGroup, Root, Source,
    component_attributes, condition, service_event,
};
use crate::journal::{IN_PROGRESS, Journal, Undo};
use crate::{Database, Table, link::Link, version};
use alloc::{
    borrow::ToOwned,
    format,
    string::{String, ToString},
    vec::Vec,
};

mod tables;
pub use tables::{IniEdit, font_title, ini_apply};

/// INSTALLMESSAGE kinds (the top byte) and MB_* styles (the low bits).
pub mod msg {
    pub const FATAL_EXIT: u32 = 0x0000_0000;
    pub const ERROR: u32 = 0x0100_0000;
    pub const WARNING: u32 = 0x0200_0000;
    pub const USER: u32 = 0x0300_0000;
    pub const INFO: u32 = 0x0400_0000;
    pub const OUT_OF_DISK_SPACE: u32 = 0x0700_0000;
    pub const ACTION_START: u32 = 0x0800_0000;
    pub const ACTION_DATA: u32 = 0x0900_0000;
    pub const PROGRESS: u32 = 0x0a00_0000;
    pub const COMMON_DATA: u32 = 0x0b00_0000;
    pub const INITIALIZE: u32 = 0x0c00_0000;
    pub const TERMINATE: u32 = 0x0d00_0000;
    pub const SHOW_DIALOG: u32 = 0x0e00_0000;
    pub const MB_OK: u32 = 0;
    pub const MB_OKCANCEL: u32 = 1;
    pub const MB_ABORTRETRYIGNORE: u32 = 2;
    pub const MB_YESNO: u32 = 4;
    pub const MB_RETRYCANCEL: u32 = 5;
    pub const MB_ICONERROR: u32 = 0x10;
    pub const MB_ICONQUESTION: u32 = 0x20;
    pub const MB_ICONWARNING: u32 = 0x30;
    pub const MB_ICONINFORMATION: u32 = 0x40;
    pub const IDOK: i32 = 1;
    pub const IDCANCEL: i32 = 2;
    pub const IDABORT: i32 = 3;
    pub const IDRETRY: i32 = 4;
    pub const IDIGNORE: i32 = 5;
    pub const IDYES: i32 = 6;
    pub const IDNO: i32 = 7;
}

/// UI levels (INSTALLUILEVEL).
pub mod level {
    pub const NONE: u32 = 2;
    pub const BASIC: u32 = 3;
    pub const REDUCED: u32 = 4;
    pub const FULL: u32 = 5;
}

/// How a package dialog ended (its EndDialog event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// EndDialog Return: the sequence continues.
    Return,
    /// EndDialog Exit: the user cancelled.
    Exit,
    /// A fatal error.
    Error,
    Retry,
    Ignore,
    /// The UI does not show package dialogs.
    Skip,
}

/// The installation's user interface.
pub trait Ui {
    /// The UI level (`level::*`).
    fn level(&self) -> u32;
    /// A message (`msg::*` kind with MB_* style bits) and its formatted
    /// text: the button chosen, or 0 when nobody answered.
    fn message(&mut self, kind: u32, text: &str) -> i32;
    /// Progress through the script: false cancels.
    fn progress(&mut self, done: u64, total: u64) -> bool {
        let _ = (done, total);
        true
    }
    /// Show a package dialog (the full UI).
    fn dialog(&mut self, session: &mut Session<'_, '_>, name: &str) -> Ending {
        let _ = (session, name);
        Ending::Skip
    }
}

/// What the session was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// msiexec /i (a fresh installation, or maintenance of the installed
    /// product).
    Install,
    /// msiexec /x.
    Remove,
    /// msiexec /f with REINSTALLMODE letters.
    Repair(String),
}

/// A custom action row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomAction {
    pub name: String,
    pub kind: i32,
    pub source: String,
    pub target: String,
}

/// Session state custom actions (MsiGetProperty, MsiSetProperty, ...) see.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub package: String,
    pub properties: Vec<(String, String)>,
    pub machine: Option<Machine>,
    pub request: Option<Request>,
    pub plan: Plan,
    /// What the installed product put down (maintenance, repair, removal).
    pub installed_plan: Option<Plan>,
    pub record: Option<records::Product>,
    pub journal: Journal,
    pub commit: Vec<(CustomAction, String)>,
    pub install_set: Vec<String>,
    pub remove_set: Vec<String>,
    /// Components of `remove_set` whose resources stay (other clients).
    pub shared: Vec<String>,
    pub reboot: bool,
    pub script: bool,
    pub nested: bool,
    pub costed: bool,
    pub done: u64,
    pub total: u64,
    pub log: Vec<String>,
    /// Files' names `[#key]` resolves to while costing is not done.
    pub serial: u64,
    /// The installation of the product finished (for the exit code and
    /// the log).
    pub executed: bool,
    /// A custom action's MsiProcessMessage answers come from the UI.
    pub cancelled: bool,
}
impl State {
    pub fn property(&self, name: &str) -> &str {
        self.properties
            .iter()
            .find(|p| p.0 == name)
            .map_or("", |p| p.1.as_str())
    }
    pub fn set_property(&mut self, name: &str, value: &str) {
        if value.is_empty() {
            self.properties.retain(|p| p.0 != name);
            return;
        }
        match self.properties.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1 = String::from(value),
            None => self
                .properties
                .push((String::from(name), String::from(value))),
        }
    }
    /// MsiFormatRecord's expansion of `text` with this session's
    /// properties, files and directories.
    pub fn format(&self, text: &str) -> String {
        let mut plan = Plan {
            properties: self.properties.clone(),
            directories: self.plan.directories.clone(),
            files: self.plan.files.clone(),
            component_dirs: self.plan.component_dirs.clone(),
            ..Plan::default()
        };
        // Directory properties win over the plan's (SetTargetPath).
        for (k, v) in &self.properties {
            if let Some(d) = plan.directories.iter_mut().find(|d| d.0 == *k) {
                d.1 = v.clone();
            }
        }
        plan.format(text)
    }
    /// Feature and component states for conditions: `&F` / `$C` the
    /// action state, `!F` / `?C` the installed state (3 local, 2 absent).
    fn condition_properties(&self) -> Vec<(String, String)> {
        let mut p = self.properties.clone();
        let installed_features: Vec<String> = self
            .record
            .as_ref()
            .map(|r| r.features.clone())
            .unwrap_or_default();
        let installed_components: Vec<String> = self
            .installed_plan
            .as_ref()
            .map(|p| p.components.clone())
            .unwrap_or_default();
        let removing = self.removing_all();
        for (feature, _) in self.feature_rows() {
            let action = if removing {
                "2"
            } else if self.plan.features.contains(&feature) {
                "3"
            } else if installed_features.contains(&feature) {
                "2"
            } else {
                "-1"
            };
            p.push((format!("&{feature}"), String::from(action)));
            let state = if installed_features.contains(&feature) {
                "3"
            } else {
                "2"
            };
            p.push((format!("!{feature}"), String::from(state)));
        }
        for c in &self.plan.component_info {
            let action = if self.install_set.contains(&c.key) {
                "3"
            } else if self.remove_set.contains(&c.key) {
                "2"
            } else {
                "-1"
            };
            p.push((format!("${}", c.key), String::from(action)));
            let state = if installed_components.contains(&c.key) {
                "3"
            } else {
                "2"
            };
            p.push((format!("?{}", c.key), String::from(state)));
        }
        p
    }
    fn feature_rows(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (f, c) in &self.plan.feature_components {
            if !out.iter().any(|x| x.0 == *f) {
                out.push((f.clone(), c.clone()));
            }
        }
        out
    }
    pub fn evaluate(&self, text: &str) -> bool {
        condition::evaluate(text, &self.condition_properties())
    }
    pub fn removing_all(&self) -> bool {
        self.property("REMOVE").eq_ignore_ascii_case("ALL")
    }
    pub fn product_code(&self) -> &str {
        self.property("ProductCode")
    }
    pub fn all_users(&self) -> bool {
        matches!(self.property("ALLUSERS"), "1" | "2")
    }
}

/// A session: the package, the machine it installs onto and its state.
pub struct Session<'s, 'd> {
    pub db: &'s Database<'d>,
    pub target: &'s mut dyn Target,
    pub state: State,
}

fn opt_table(db: &Database, name: &str) -> Option<Table> {
    db.table(name).ok()
}

/// Windows Installer's text for its own error messages.
pub fn error_text(number: u32) -> &'static str {
    match number {
        1310 => "Error writing to file: [2]. Verify that you have access to that directory.",
        1311 => {
            "Source file not found: [2]. Verify that the file exists and that you can access it."
        }
        1335 => {
            "The cabinet file '[2]' required for this installation is corrupt and cannot be used. This could indicate a network error, an error reading from the CD-ROM, or a problem with this package."
        }
        1406 => {
            "Could not write value [2] to key [3]. Verify that you have sufficient access to that key, or contact your support personnel."
        }
        1638 => {
            "Another version of this product is already installed. Installation of this version cannot continue. To configure or remove the existing version of this product, use Add/Remove Programs on the Control Panel."
        }
        1721 => {
            "There is a problem with this Doors Installer package. A program required for this install to complete could not be run. Contact your support personnel or package vendor. Action: [2], location: [3], command: [4]"
        }
        1722 => {
            "There is a problem with this Doors Installer package. A program run as part of the setup did not finish as expected. Contact your support personnel or package vendor. Action [2], location: [3], command: [4]"
        }
        1723 => {
            "There is a problem with this Doors Installer package. A DLL required for this install to complete could not be run. Contact your support personnel or package vendor. Action [2], entry: [3], library: [4]"
        }
        1904 => "Module [2] failed to register.  HRESULT [3].  Contact your support personnel.",
        1905 => "Module [2] failed to unregister.  HRESULT [3].  Contact your support personnel.",
        1911 => "Could not register type library for file [2].  Contact your support personnel.",
        1920 => {
            "Service '[2]' ([3]) failed to start. Verify that you have sufficient privileges to start system services."
        }
        1923 => {
            "Service '[2]' ([3]) could not be installed. Verify that you have sufficient privileges to install system services."
        }
        2738 => "Could not access VBScript run time for custom action [2].",
        2739 => "Could not access JScript run time for custom action [2].",
        2762 => "Cannot write script record. Transaction not started.",
        _ => "",
    }
}
/// `[1]`.. placeholders of an error text filled.
fn fill(template: &str, fields: &[&str]) -> String {
    let mut out = String::from(template);
    for (i, f) in fields.iter().enumerate().rev() {
        out = out.replace(&format!("[{}]", i + 2), f);
    }
    out
}

/// The description an action shows while it runs (the ActionText table's,
/// or Windows Installer's own).
pub fn action_text(db: &Database, action: &str) -> String {
    if let Some(t) = opt_table(db, "ActionText")
        && let Some(r) = t.find(action)
        && let Some(d) = t.get(r, "Description").str()
    {
        return String::from(d);
    }
    String::from(match action {
        "InstallFiles" => "Copying new files",
        "RemoveFiles" => "Removing files",
        "WriteRegistryValues" => "Writing system registry values",
        "RemoveRegistryValues" => "Removing system registry values",
        "CreateShortcuts" => "Creating shortcuts",
        "RemoveShortcuts" => "Removing shortcuts",
        "RegisterProduct" => "Registering product",
        "PublishProduct" => "Publishing product information",
        "UnpublishProduct" => "Unpublishing product information",
        "PublishFeatures" => "Publishing Product Features",
        "UnpublishFeatures" => "Unpublishing Product Features",
        "InstallServices" => "Installing new services",
        "StartServices" => "Starting services",
        "StopServices" => "Stopping services",
        "DeleteServices" => "Deleting services",
        "CreateFolders" => "Creating folders",
        "RemoveFolders" => "Removing folders",
        "ProcessComponents" => "Updating component registration",
        "RegisterUser" => "Registering user",
        "InstallValidate" => "Validating install",
        "CostInitialize" | "FileCost" | "CostFinalize" => "Computing space requirements",
        "RemoveExistingProducts" => "Removing applications",
        "FindRelatedProducts" => "Searching for related applications",
        "AppSearch" => "Searching for installed applications",
        "LaunchConditions" => "Evaluating launch conditions",
        "RegisterExtensionInfo" => "Registering extension servers",
        "RegisterProgIdInfo" => "Registering program identifiers",
        "UnregisterExtensionInfo" => "Unregistering extension servers",
        "UnregisterProgIdInfo" => "Unregistering program identifiers",
        "Rollback" => "Rolling back action:",
        _ => "",
    })
}

/// Windows Installer's version comparison of products: the first three
/// fields.
fn product_version(text: &str) -> u64 {
    version::parse(text).unwrap_or(0) & !0xffff
}

impl<'s, 'd> Session<'s, 'd> {
    /// A session for `request` on the package at `package` (a Windows
    /// path), with the command line's `properties`.
    pub fn new(
        db: &'s Database<'d>,
        target: &'s mut dyn Target,
        package: &str,
        request: Request,
        machine: Machine,
        properties: &[(String, String)],
        ui_level: u32,
    ) -> Self {
        let mut state = State {
            package: String::from(package),
            request: Some(request.clone()),
            ..State::default()
        };
        for (k, v) in &machine.properties {
            state.set_property(k, v);
        }
        if let Some(t) = opt_table(db, "Property") {
            for r in 0..t.rows.len() {
                if let (Some(k), Some(v)) = (t.get(r, "Property").str(), t.get(r, "Value").str()) {
                    state.set_property(k, v);
                }
            }
        }
        let folder = package.rfind('\\').map_or("", |i| &package[..=i]);
        state.set_property("OriginalDatabase", package);
        state.set_property("DATABASE", package);
        state.set_property("SourceDir", folder);
        state.set_property("SOURCEDIR", folder);
        state.set_property("UILevel", &format!("{ui_level}"));
        state.set_property("ACTION", "INSTALL");
        if let Ok(s) = db.summary()
            && let Some(code) = s.text(crate::summary::pid::REVISION)
        {
            state.set_property("PackageCode", code);
        }
        // The installed product (and what it remembered).
        let product = String::from(state.property("ProductCode"));
        let record = records::product(&mut *target, &product);
        if let Some(r) = &record {
            state.set_property(
                "Installed",
                if r.install_date.is_empty() {
                    "1"
                } else {
                    &r.install_date
                },
            );
            state.set_property("ProductState", "5");
            for (k, v) in &r.properties {
                state.set_property(k, v);
            }
        } else {
            state.set_property("ProductState", "-1");
        }
        match &request {
            Request::Install => (),
            Request::Remove => state.set_property("REMOVE", "ALL"),
            Request::Repair(mode) => {
                state.set_property("REINSTALL", "ALL");
                state.set_property("REINSTALLMODE", mode);
            }
        }
        for (k, v) in properties {
            state.set_property(k, v);
        }
        if state.property("REINSTALLMODE").is_empty() {
            state.set_property("REINSTALLMODE", "omus");
        }
        let elevated = target.elevated();
        state.set_property("Privileged", if elevated { "1" } else { "" });
        state.set_property("AdminUser", if elevated { "1" } else { "" });
        // ALLUSERS=2: per-machine for an administrator, else per-user.
        if state.property("ALLUSERS") == "2" && !elevated {
            state.set_property("ALLUSERS", "");
        }
        state.machine = Some(machine);
        state.record = record;
        state.serial = {
            let mut h: u64 = target.boot() ^ 0xcbf2_9ce4_8422_2325;
            for b in product.bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
            h & 0xffff_ffff
        };
        Self { db, target, state }
    }

    pub fn property(&self, name: &str) -> &str {
        self.state.property(name)
    }
    pub fn set_property(&mut self, name: &str, value: &str) {
        self.state.set_property(name, value);
    }
    pub fn format(&self, text: &str) -> String {
        self.state.format(text)
    }
    pub fn evaluate(&self, text: &str) -> bool {
        self.state.evaluate(text)
    }
    pub fn log(&mut self, line: &str) {
        self.target.log(line);
        self.state.log.push(String::from(line));
    }
    fn record(&mut self, undo: Undo) -> Result<(), u32> {
        self.state.journal.record(&mut *self.target, undo)
    }
    /// Show an error (Windows Installer's message `number` with its fields)
    /// and get the user's answer.
    fn error(&mut self, ui: &mut dyn Ui, number: u32, fields: &[&str], style: u32) -> i32 {
        let text = self.error_message(number, fields);
        self.log(&format!("Error {number}. {text}"));
        ui.message(
            msg::ERROR | style | msg::MB_ICONERROR,
            &format!("Error {number}. {text}"),
        )
    }
    /// A warning (message `number`): logged and shown, and the
    /// installation goes on.
    fn warning(&mut self, ui: &mut dyn Ui, number: u32, fields: &[&str]) {
        let text = self.error_message(number, fields);
        self.log(&format!("Warning {number}. {text}"));
        ui.message(
            msg::WARNING | msg::MB_OK | msg::MB_ICONWARNING,
            &format!("Warning {number}. {text}"),
        );
    }
    fn error_message(&self, number: u32, fields: &[&str]) -> String {
        if let Some(t) = opt_table(self.db, "Error") {
            for r in 0..t.rows.len() {
                if t.get(r, "Error").int() == Some(number as i32)
                    && let Some(m) = t.get(r, "Message").str()
                {
                    return self.format(&fill(m, fields));
                }
            }
        }
        fill(error_text(number), fields)
    }

    // ---- the session -----------------------------------------------------

    /// Carry out the request: the exit code msiexec returns.
    pub fn run(&mut self, ui: &mut dyn Ui) -> u32 {
        if let Err(e) = self.recover_interrupted(ui) {
            return e;
        }
        let product = String::from(self.state.product_code());
        if product.is_empty() {
            return code::BAD_PACKAGE;
        }
        let request = self.state.request.clone().unwrap_or(Request::Install);
        let installed = self.state.record.is_some();
        match &request {
            Request::Remove | Request::Repair(_) if !installed => {
                return code::UNKNOWN_PRODUCT;
            }
            Request::Install if installed => {
                // The same product from a different package: another
                // version of it is installed.
                let package_code = self.property("PackageCode").to_string();
                let recorded = self
                    .state
                    .record
                    .as_ref()
                    .map(|r| r.package_code.clone())
                    .unwrap_or_default();
                let reinstall = !self.property("REINSTALL").is_empty();
                if !recorded.is_empty()
                    && !package_code.eq_ignore_ascii_case(&recorded)
                    && !reinstall
                    && !self.property("REINSTALLMODE").contains('v')
                {
                    self.error(ui, 1638, &[], msg::MB_OK);
                    return code::PRODUCT_VERSION;
                }
            }
            _ => (),
        }
        self.log(&format!(
            "Product: {} -- {} started.",
            self.property("ProductName"),
            match request {
                Request::Remove => "Removal",
                Request::Repair(_) => "Repair",
                Request::Install if installed => "Configuration",
                Request::Install => "Installation",
            }
        ));
        let ui_sequence = ui.level() >= level::REDUCED
            && self.db.tables.iter().any(|t| t.0 == "InstallUISequence");
        let result = if ui_sequence {
            self.sequence(ui, "InstallUISequence")
        } else {
            self.execute(ui)
        };
        let status = match result {
            Ok(()) => code::SUCCESS,
            Err(code::NO_MORE_ITEMS) => code::SUCCESS,
            Err(e) => e,
        };
        // The end dialog of the full UI: success, user exit or fatal error.
        if ui_sequence && status != code::ELEVATION_REQUIRED {
            let which = match status {
                code::SUCCESS => -1,
                code::INSTALL_USEREXIT => -2,
                _ => -3,
            };
            if let Some(dialog) = self.end_dialog(which) {
                ui.dialog(self, &dialog);
            }
        }
        let status = if status == code::SUCCESS && self.state.reboot {
            self.reboot(ui)
        } else {
            status
        };
        let verb = match request {
            Request::Remove => "Removal",
            Request::Repair(_) => "Configuration",
            Request::Install => "Installation",
        };
        let outcome = match status {
            0 | code::SUCCESS_REBOOT_REQUIRED | code::SUCCESS_REBOOT_INITIATED => {
                format!("{verb} completed successfully.")
            }
            code::INSTALL_USEREXIT => format!("{verb} operation was cancelled."),
            _ => format!("{verb} failed."),
        };
        self.log(&format!(
            "Product: {} -- {outcome}",
            self.property("ProductName")
        ));
        self.log(&format!(
            "Doors Installer {} the product. Product Name: {}. Product Version: {}. Product Language: {}. Manufacturer: {}. Installation success or error status: {status}.",
            match request {
                Request::Remove => "removed",
                Request::Repair(_) => "reconfigured",
                Request::Install => "installed",
            },
            self.property("ProductName"),
            self.property("ProductVersion"),
            self.property("ProductLanguage"),
            self.property("Manufacturer"),
        ));
        status
    }

    /// A package's end dialog (sequence -1, -2 or -3).
    fn end_dialog(&self, which: i32) -> Option<String> {
        let t = opt_table(self.db, "InstallUISequence")?;
        (0..t.rows.len())
            .find(|&r| t.get(r, "Sequence").int() == Some(which))
            .and_then(|r| {
                let condition = t.get(r, "Condition").str().unwrap_or("");
                self.evaluate(condition)
                    .then(|| t.get(r, "Action").str().map(String::from))?
            })
    }

    fn reboot(&mut self, ui: &mut dyn Ui) -> u32 {
        let reboot = self.property("REBOOT").to_ascii_uppercase();
        if reboot.starts_with('R') || reboot.starts_with('S') || ui.level() <= level::NONE {
            return code::SUCCESS_REBOOT_REQUIRED;
        }
        let text = self.format(
            "You must restart your system for the configuration changes made to [ProductName] to take effect. Click Yes to restart now or No if you plan to manually restart later.",
        );
        if ui.message(msg::USER | msg::MB_YESNO | msg::MB_ICONINFORMATION, &text) == msg::IDYES {
            code::SUCCESS_REBOOT_INITIATED
        } else {
            code::SUCCESS_REBOOT_REQUIRED
        }
    }

    /// The InstallExecuteSequence, after the elevation check.
    pub fn execute(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        if self.state.all_users() && !self.target.elevated() {
            return Err(if ui.level() <= level::NONE {
                code::INSUFFICIENT_PRIVILEGES
            } else {
                code::ELEVATION_REQUIRED
            });
        }
        let result = self.sequence(ui, "InstallExecuteSequence");
        match result {
            Ok(()) | Err(code::NO_MORE_ITEMS) => {
                if self.state.script {
                    // A sequence without InstallFinalize commits at its end.
                    self.install_finalize(ui)?;
                }
                self.state.executed = true;
                Ok(())
            }
            Err(e) => {
                if self.state.script {
                    self.rollback(ui);
                }
                Err(e)
            }
        }
    }

    /// Run a sequence table's actions whose conditions hold, in order.
    pub fn sequence(&mut self, ui: &mut dyn Ui, table: &str) -> Result<(), u32> {
        let Some(t) = opt_table(self.db, table) else {
            return Ok(());
        };
        let mut rows: Vec<(i32, String, String)> = (0..t.rows.len())
            .filter_map(|r| {
                let seq = t.get(r, "Sequence").int()?;
                (seq > 0).then(|| {
                    (
                        seq,
                        t.get(r, "Action").str().unwrap_or("").to_string(),
                        t.get(r, "Condition").str().unwrap_or("").to_string(),
                    )
                })
            })
            .collect();
        rows.sort_by_key(|r| r.0);
        let handoff = self.property("DOORS_UI_HANDOFF") == "1" && table == "InstallUISequence";
        for (_, action, cond) in rows {
            if !self.evaluate(&cond) {
                continue;
            }
            // An elevated continuation of the UI skips the dialogs the user
            // already answered.
            if handoff && self.is_dialog(&action) && self.dialog_is_modal(&action) {
                continue;
            }
            self.action(ui, &action)?;
        }
        Ok(())
    }

    pub fn is_dialog(&self, name: &str) -> bool {
        opt_table(self.db, "Dialog").is_some_and(|t| t.find(name).is_some())
    }
    fn dialog_is_modal(&self, name: &str) -> bool {
        opt_table(self.db, "Dialog")
            .and_then(|t| t.find(name).and_then(|r| t.get(r, "Attributes").int()))
            .is_some_and(|a| a & 0x2 != 0)
    }
    pub fn custom_action(&self, name: &str) -> Option<CustomAction> {
        let t = opt_table(self.db, "CustomAction")?;
        let r = t.find(name)?;
        Some(CustomAction {
            name: String::from(name),
            kind: t.get(r, "Type").int().unwrap_or(0),
            source: t.get(r, "Source").str().unwrap_or("").to_string(),
            target: t.get(r, "Target").str().unwrap_or("").to_string(),
        })
    }

    /// One action: a dialog, a custom action or a standard action
    /// (MsiDoAction).
    pub fn action(&mut self, ui: &mut dyn Ui, name: &str) -> Result<(), u32> {
        if self.is_dialog(name) {
            return match ui.dialog(self, name) {
                Ending::Return | Ending::Skip | Ending::Ignore => Ok(()),
                Ending::Exit => Err(code::INSTALL_USEREXIT),
                Ending::Error | Ending::Retry => Err(code::INSTALL_FAILURE),
            };
        }
        let text = action_text(self.db, name);
        ui.message(msg::ACTION_START, &format!("{name}\t{text}"));
        self.log(&format!("Action start: {name}."));
        let result = match self.custom_action(name) {
            Some(ca) => self.custom(ui, &ca),
            None => self.standard(ui, name),
        };
        let value = match result {
            Ok(()) => 1,
            Err(code::INSTALL_USEREXIT) => 2,
            Err(code::NO_MORE_ITEMS) => 1,
            Err(e) => {
                self.log(&format!("DOOR_P16 MSI ACTION failed name={name} code={e}"));
                3
            }
        };
        self.log(&format!("Action ended: {name}. Return value {value}."));
        result
    }

    /// A standard action whose table Doors does not carry out: fails when
    /// the package has rows for it.
    fn unsupported(&mut self, name: &str) -> Result<(), u32> {
        let tables: &[&str] = match name {
            "PatchFiles" => &["Patch"],
            "SetODBCFolders" | "InstallODBC" | "RemoveODBC" => {
                &["ODBCDriver", "ODBCTranslator", "ODBCDataSource"]
            }
            // Side-by-side and .NET assemblies (WinSxS, the GAC), COM+
            // applications, protected-file catalogs and the services'
            // extended configuration.
            "MsiPublishAssemblies" | "MsiUnpublishAssemblies" => &["MsiAssembly"],
            "RegisterComPlus" | "UnregisterComPlus" => &["Complus"],
            "InstallSFPCatalogFile" => &["SFPCatalog"],
            "MsiConfigureServices" => &["MsiServiceConfig", "MsiServiceConfigFailureActions"],
            // An administrative installation is not offered.
            _ => &[""],
        };
        let used = tables
            .iter()
            .find(|t| t.is_empty() || opt_table(self.db, t).is_some_and(|t| !t.rows.is_empty()));
        match used {
            None => Ok(()),
            Some(table) => {
                self.log(&format!(
                    "Action {name}: the {table} table is not supported by Doors Installer; the installation stops."
                ));
                Err(crate::execute::code::INSTALL_FAILURE)
            }
        }
    }
    fn standard(&mut self, ui: &mut dyn Ui, name: &str) -> Result<(), u32> {
        match name {
            "LaunchConditions" => self.launch_conditions(ui),
            "AppSearch" => {
                self.app_search();
                Ok(())
            }
            "FindRelatedProducts" => {
                self.find_related();
                Ok(())
            }
            "CostInitialize" | "FileCost" => Ok(()),
            "CostFinalize" => self.cost_finalize(),
            "InstallValidate" => self.install_validate(ui),
            "InstallInitialize" => self.install_initialize(),
            "RemoveExistingProducts" => self.remove_existing(ui),
            "ProcessComponents" => self.process_components(),
            "StopServices" => self.stop_services(ui),
            "DeleteServices" => self.delete_services(),
            "UnpublishFeatures" => self.unpublish_features(),
            "RemoveRegistryValues" => self.remove_registry(RegistryGroup::Registry),
            "RemoveShortcuts" => self.remove_shortcuts(),
            "RemoveFiles" => self.remove_files(ui),
            "RemoveFolders" => self.remove_folders(),
            "CreateFolders" => self.create_folders(),
            "InstallFiles" => self.install_files(ui),
            "WriteRegistryValues" => self.write_registry(ui, RegistryGroup::Registry),
            "RegisterExtensionInfo" => self.write_registry(ui, RegistryGroup::Extension),
            "RegisterProgIdInfo" => self.write_registry(ui, RegistryGroup::ProgId),
            "RegisterClassInfo" => self.write_registry(ui, RegistryGroup::Class),
            "RegisterMIMEInfo" => self.write_registry(ui, RegistryGroup::Mime),
            "PublishComponents" => self.write_registry(ui, RegistryGroup::Published),
            "UnregisterExtensionInfo" => self.remove_registry(RegistryGroup::Extension),
            "UnregisterProgIdInfo" => self.remove_registry(RegistryGroup::ProgId),
            "UnregisterClassInfo" => self.remove_registry(RegistryGroup::Class),
            "UnregisterMIMEInfo" => self.remove_registry(RegistryGroup::Mime),
            "UnpublishComponents" => self.remove_registry(RegistryGroup::Published),
            "RegisterFonts" => self.register_fonts(),
            "UnregisterFonts" => self.unregister_fonts(),
            "RegisterTypeLibraries" => self.register_typelibs(ui, true),
            "UnregisterTypeLibraries" => self.register_typelibs(ui, false),
            "SelfRegModules" => self.self_register(ui, true),
            "SelfUnregModules" => self.self_register(ui, false),
            "WriteEnvironmentStrings" => self.write_environment(),
            "RemoveEnvironmentStrings" => self.remove_environment(),
            "WriteIniValues" => self.write_ini(),
            "RemoveIniValues" => self.remove_ini(),
            "DuplicateFiles" => self.duplicate_files(ui),
            "RemoveDuplicateFiles" => self.remove_duplicate_files(),
            "MoveFiles" => self.move_files(ui),
            // Planned with the components: InstallFiles copies an isolated
            // component's files into its application's folder.
            "IsolateComponents" => Ok(()),
            "CreateShortcuts" => self.create_shortcuts(),
            "InstallServices" => self.install_services(ui),
            "StartServices" => self.start_services(ui),
            "RegisterUser" => Ok(()),
            "RegisterProduct" => self.register_product(),
            "PublishFeatures" => self.publish_features(),
            "PublishProduct" => self.publish_product(),
            "InstallFinalize" => self.install_finalize(ui),
            "ScheduleReboot" => {
                self.state.reboot = true;
                Ok(())
            }
            "ForceReboot" => {
                self.state.reboot = true;
                Ok(())
            }
            "ExecuteAction" => self.execute(ui),
            "InstallExecute" | "InstallExecuteAgain" => Ok(()),
            // Nothing to do on Doors (registry space, image binding,
            // source resolution).
            "MigrateFeatureStates"
            | "ValidateProductID"
            | "AllocateRegistrySpace"
            | "CCPSearch"
            | "RMCCPSearch"
            | "ResolveSource"
            | "BindImage"
            | "CreateFoldersAgain"
            | "SetFeatureStates" => Ok(()),
            // Actions whose tables Doors' installer does not carry out
            // (patching, ODBC, administrative installation): a package
            // that has rows for them fails (1603, with the reason in the
            // log) rather than reporting an installation it did not make;
            // without rows there is nothing to do.
            "PatchFiles"
            | "SetODBCFolders"
            | "InstallODBC"
            | "RemoveODBC"
            | "InstallAdminPackage"
            | "MsiPublishAssemblies"
            | "MsiUnpublishAssemblies"
            | "RegisterComPlus"
            | "UnregisterComPlus"
            | "InstallSFPCatalogFile"
            | "MsiConfigureServices" => self.unsupported(name),
            _ => {
                self.log(&format!("Action {name} is not a standard action; skipped."));
                Ok(())
            }
        }
    }

    // ---- recovery ----------------------------------------------------------

    /// An installation still recorded in progress: from this boot, another
    /// is running (1618); from an earlier boot it was interrupted and is
    /// rolled back first.
    fn recover_interrupted(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        if self.state.nested {
            return Ok(());
        }
        let text = |v: Option<RegValue>| match v {
            Some(RegValue::String(s)) => s,
            _ => String::new(),
        };
        let script = text(self.target.value(Root::LocalMachine, IN_PROGRESS, "Script"));
        if script.is_empty() {
            return Ok(());
        }
        let boot = text(self.target.value(Root::LocalMachine, IN_PROGRESS, "Boot"));
        // The installation that holds its script open is still running; one
        // that does not (a crashed process, an earlier boot) was
        // interrupted.
        let running = self
            .target
            .in_use(&script)
            .unwrap_or_else(|| boot == format!("{}", self.target.boot()));
        if running {
            return Err(code::INSTALL_ALREADY_RUNNING);
        }
        let product = text(
            self.target
                .value(Root::LocalMachine, IN_PROGRESS, "Product"),
        );
        let journal = Journal::load(&mut *self.target, &script);
        let (entries, nontransactional) = journal
            .as_ref()
            .map(|j| {
                (
                    j.entries.len(),
                    j.entries
                        .iter()
                        .filter(|e| matches!(e, Undo::Nontransactional(_)))
                        .count(),
                )
            })
            .unwrap_or((0, 0));
        self.log(&format!(
            "Rolling back the interrupted installation of {product} ({entries} entries)."
        ));
        if let Some(j) = journal {
            self.state.journal = j;
            self.rollback(ui);
        } else {
            self.target.delete_key(Root::LocalMachine, IN_PROGRESS);
        }
        self.log(&format!(
            "DOOR_P16 MSI ROLLBACK interrupted product={product} entries={entries} nontransactional={nontransactional}"
        ));
        Ok(())
    }

    /// Undo the journal backwards; the InProgress record and the script go.
    pub fn rollback(&mut self, ui: &mut dyn Ui) {
        ui.message(msg::ACTION_START, "Rollback\tRolling back action:");
        self.log("Action start: Rollback.");
        let entries = core::mem::take(&mut self.state.journal.entries);
        for undo in entries.iter().rev() {
            match undo {
                Undo::Created(p) => {
                    if self.target.exists(p) {
                        let _ = self.target.remove_file(p);
                    }
                }
                Undo::Replaced { path, backup } => {
                    if self.target.exists(backup) {
                        if self.target.exists(path) {
                            let _ = self.target.remove_file(path);
                        }
                        let _ = self.target.rename(backup, path);
                    }
                }
                Undo::Folder(p) => {
                    let _ = self.target.remove_folder(p);
                }
                Undo::FolderRemoved(p) => {
                    if !self.target.exists(p) {
                        let _ = self.target.create_folder(p);
                    }
                }
                Undo::Value {
                    root,
                    key,
                    name,
                    previous,
                } => {
                    let n = (!name.is_empty()).then_some(name.as_str());
                    match previous {
                        Some(v) => {
                            let _ = self.target.set_value(*root, key, n, v);
                        }
                        None => self.target.delete_value(*root, key, n),
                    }
                }
                Undo::Key { root, key } => self.target.delete_key(*root, key),
                Undo::Service(n) => {
                    let _ = self.target.stop_service(n, true);
                    let _ = self.target.delete_service(n);
                }
                Undo::ServiceChanged(n) => {
                    self.log(&format!("Rollback could not restore service {n}."));
                }
                Undo::Action {
                    name,
                    kind,
                    source,
                    target,
                    data,
                } => {
                    let ca = CustomAction {
                        name: name.clone(),
                        kind: kind & !0x500,
                        source: source.clone(),
                        target: target.clone(),
                    };
                    self.log(&format!("Rollback custom action {name}."));
                    let _ = self.run_custom(ui, &ca, Some(data.clone()));
                }
                Undo::Nontransactional(n) => {
                    self.log(&format!(
                        "Rollback could not undo custom action {n}: it changed the system outside the installation script."
                    ));
                }
                Undo::Registration {
                    kind,
                    path,
                    help,
                    registered,
                    per_user,
                    view32,
                } => {
                    // The opposite call.
                    let result = match kind {
                        crate::journal::Registration::TypeLib => self.target.register_typelib(
                            path,
                            help,
                            *per_user,
                            *view32,
                            !registered,
                        ),
                        crate::journal::Registration::Module => {
                            self.target.self_register(path, !registered)
                        }
                    };
                    if let Err(e) = result {
                        self.log(&format!(
                            "Rollback could not restore the registration of {path}: {e:#x}."
                        ));
                    }
                }
            }
        }
        let journal_path = self.state.journal.path.clone();
        if !journal_path.is_empty() && self.target.exists(&journal_path) {
            let _ = self.target.remove_file(&journal_path);
        }
        if !self.state.nested {
            let _ = self.target.remove_folder(crate::journal::FOLDER);
        }
        self.state.journal = Journal::default();
        self.state.script = false;
        if !self.state.nested {
            self.target.delete_key(Root::LocalMachine, IN_PROGRESS);
        }
        self.log("Action ended: Rollback. Return value 3.");
    }

    // ---- journaled operations --------------------------------------------

    fn j_folder_tree(&mut self, folder: &str) -> Result<(), u32> {
        let trimmed = folder.trim_end_matches('\\');
        let mut prefixes = Vec::new();
        let mut at = 0;
        while let Some(i) = trimmed[at..].find('\\') {
            prefixes.push(String::from(&trimmed[..at + i]));
            at += i + 1;
        }
        prefixes.push(String::from(trimmed));
        for p in prefixes {
            if p.ends_with(':') || p.is_empty() || self.target.exists(&p) {
                continue;
            }
            self.record(Undo::Folder(p.clone()))?;
            self.target.create_folder(&p)?;
        }
        Ok(())
    }
    fn j_write_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32> {
        if let Some(i) = path.rfind('\\') {
            self.j_folder_tree(&path[..i])?;
        }
        if self.target.exists(path) {
            let backup = self.state.journal.backup_name();
            self.record(Undo::Replaced {
                path: String::from(path),
                backup: backup.clone(),
            })?;
            self.target.rename(path, &backup)?;
        } else {
            self.record(Undo::Created(String::from(path)))?;
        }
        self.target.write_file(path, data)?;
        self.target.stamp(path);
        Ok(())
    }
    fn j_remove_file(&mut self, path: &str) -> Result<(), u32> {
        if !self.target.exists(path) {
            return Ok(());
        }
        if !self.state.journal.active() {
            return self.target.remove_file(path);
        }
        let backup = self.state.journal.backup_name();
        self.record(Undo::Replaced {
            path: String::from(path),
            backup: backup.clone(),
        })?;
        self.target.rename(path, &backup)
    }
    fn j_remove_folder(&mut self, path: &str) -> bool {
        let path = path.trim_end_matches('\\');
        if path.ends_with(':') || !self.target.exists(path) {
            return false;
        }
        if self
            .record(Undo::FolderRemoved(String::from(path)))
            .is_err()
        {
            return false;
        }
        self.target.remove_folder(path).is_ok()
    }
    fn key_exists(&mut self, root: Root, key: &str) -> bool {
        !self.target.values(root, key).is_empty() || !self.target.subkeys(root, key).is_empty()
    }
    fn j_set_value(
        &mut self,
        root: Root,
        key: &str,
        name: Option<&str>,
        value: &RegValue,
    ) -> Result<(), u32> {
        if !self.key_exists(root, key) {
            self.record(Undo::Key {
                root,
                key: String::from(key),
            })?;
        }
        let previous = self.target.value(root, key, name.unwrap_or(""));
        self.record(Undo::Value {
            root,
            key: String::from(key),
            name: String::from(name.unwrap_or("")),
            previous,
        })?;
        self.target.set_value(root, key, name, value)
    }
    fn j_delete_value(&mut self, root: Root, key: &str, name: Option<&str>) -> Result<(), u32> {
        let Some(previous) = self.target.value(root, key, name.unwrap_or("")) else {
            return Ok(());
        };
        self.record(Undo::Value {
            root,
            key: String::from(key),
            name: String::from(name.unwrap_or("")),
            previous: Some(previous),
        })?;
        self.target.delete_value(root, key, name);
        Ok(())
    }
    /// Delete a key, its values recorded so rollback recreates them.
    fn j_delete_key(&mut self, root: Root, key: &str) -> Result<(), u32> {
        for child in self.target.subkeys(root, key) {
            self.j_delete_key(root, &format!("{key}\\{child}"))?;
        }
        for (name, value) in self.target.values(root, key) {
            self.record(Undo::Value {
                root,
                key: String::from(key),
                name,
                previous: Some(value),
            })?;
        }
        self.target.delete_key(root, key);
        Ok(())
    }

    // ---- search and costing ----------------------------------------------

    fn launch_conditions(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        let Some(t) = opt_table(self.db, "LaunchCondition") else {
            return Ok(());
        };
        for r in 0..t.rows.len() {
            let cond = t.get(r, "Condition").str().unwrap_or("");
            if !self.evaluate(cond) {
                let text = self.format(t.get(r, "Description").str().unwrap_or(""));
                self.log(&text);
                ui.message(msg::ERROR | msg::MB_OK | msg::MB_ICONWARNING, &text);
                return Err(code::INSTALL_FAILURE);
            }
        }
        Ok(())
    }

    /// AppSearch: RegLocator, DrLocator and CompLocator signatures.
    fn app_search(&mut self) {
        let Some(search) = opt_table(self.db, "AppSearch") else {
            return;
        };
        let reg = opt_table(self.db, "RegLocator");
        let ini = opt_table(self.db, "IniLocator");
        let dir = opt_table(self.db, "DrLocator");
        let comp = opt_table(self.db, "CompLocator");
        for r in 0..search.rows.len() {
            let property = search.get(r, "Property").str().unwrap_or("").to_string();
            let signature = search.get(r, "Signature_").str().unwrap_or("").to_string();
            let mut found: Option<String> = None;
            if let Some(t) = &reg
                && let Some(row) = t.find(&signature)
            {
                // msidbLocatorType64bit (0x10) searches the 64-bit view;
                // without it, the 32-bit one.
                let raw_type = t.get(row, "Type").int().unwrap_or(0);
                let root = match t.get(row, "Root").int().unwrap_or(2) {
                    0 => Root::ClassesRoot,
                    1 => Root::CurrentUser,
                    3 => Root::Users,
                    _ => Root::LocalMachine,
                }
                .in_view(raw_type & 0x10 == 0);
                let key = self.format(t.get(row, "Key").str().unwrap_or(""));
                let name = self.format(t.get(row, "Name").str().unwrap_or(""));
                let kind = raw_type & 0xf;
                if let Some(v) = self.target.value(root, &key, &name) {
                    let raw = match &v {
                        RegValue::String(s) | RegValue::Expand(s) => s.clone(),
                        RegValue::Dword(d) => format!("#{d}"),
                        RegValue::Binary(b) => {
                            let mut s = String::from("#x");
                            for x in b {
                                s.push_str(&format!("{x:02X}"));
                            }
                            s
                        }
                        RegValue::Multi(m) => m.join("[~]"),
                    };
                    found = match kind {
                        // Raw value.
                        2 => Some(raw),
                        // A directory or file path that must exist.
                        _ => {
                            let path = raw.trim_matches('"').to_string();
                            self.target.exists(path.trim_end_matches('\\')).then(|| {
                                if kind == 0 && !path.ends_with('\\') {
                                    format!("{path}\\")
                                } else {
                                    path
                                }
                            })
                        }
                    };
                }
            }
            // IniLocator: a value of an INI file in the Windows folder (its
            // Field-th comma-separated field when Field is set): raw (type
            // 2), or a folder (0) or file path (1) that must exist.
            if found.is_none()
                && let Some(t) = &ini
                && let Some(row) = t.find(&signature)
            {
                let name = t.get(row, "FileName").str().unwrap_or("");
                let name = name.rsplit('|').next().unwrap_or(name);
                let windows = String::from(self.property("WindowsFolder"));
                let windows = if windows.is_empty() {
                    String::from("C:\\Windows\\")
                } else {
                    windows
                };
                let path = format!("{windows}{name}");
                if let Ok(bytes) = self.target.read_file(&path) {
                    let section = t.get(row, "Section").str().unwrap_or("");
                    let key = t.get(row, "Key").str().unwrap_or("");
                    let field = t.get(row, "Field").int().unwrap_or(0);
                    let kind = t.get(row, "Type").int().unwrap_or(1) & 0xf;
                    let value = tables::ini_value(&tables::decode_ini(&bytes).0, section, key)
                        .and_then(|v| match field {
                            n if n > 0 => v
                                .split(',')
                                .nth(n as usize - 1)
                                .map(|f| String::from(f.trim())),
                            _ => Some(v),
                        });
                    found = value.and_then(|v| match kind {
                        2 => Some(v),
                        _ => {
                            let p = String::from(v.trim_matches('"'));
                            self.target.exists(p.trim_end_matches('\\')).then(|| {
                                if kind == 0 && !p.ends_with('\\') {
                                    format!("{p}\\")
                                } else {
                                    p
                                }
                            })
                        }
                    });
                }
            }
            if found.is_none()
                && let Some(t) = &dir
                && let Some(row) = t.find(&signature)
            {
                let path = self.format(t.get(row, "Path").str().unwrap_or(""));
                let parent = t.get(row, "Parent").str().unwrap_or("");
                let base = if parent.is_empty() {
                    String::new()
                } else {
                    self.property(parent).to_string()
                };
                let full = format!("{base}{path}");
                if !full.is_empty() && self.target.exists(full.trim_end_matches('\\')) {
                    found = Some(if full.ends_with('\\') {
                        full
                    } else {
                        format!("{full}\\")
                    });
                }
            }
            if found.is_none()
                && let Some(t) = &comp
                && let Some(row) = t.find(&signature)
            {
                let id = t.get(row, "ComponentId").str().unwrap_or("").to_string();
                if let Some((_, path)) = records::clients(&mut *self.target, &id).into_iter().next()
                {
                    let folder = match t.get(row, "Type").int().unwrap_or(1) {
                        0 => path
                            .rfind('\\')
                            .map(|i| String::from(&path[..=i]))
                            .unwrap_or(path),
                        _ => path,
                    };
                    found = Some(folder);
                }
            }
            if let Some(v) = found {
                self.set_property(&property, &v);
            }
        }
    }

    /// FindRelatedProducts: installed products of the Upgrade table's
    /// upgrade codes and version ranges, listed in each row's
    /// ActionProperty.
    fn find_related(&mut self) {
        let Some(t) = opt_table(self.db, "Upgrade") else {
            return;
        };
        let own = String::from(self.state.product_code());
        for r in 0..t.rows.len() {
            let upgrade = t.get(r, "UpgradeCode").str().unwrap_or("").to_string();
            let property = t.get(r, "ActionProperty").str().unwrap_or("").to_string();
            let attributes = t.get(r, "Attributes").int().unwrap_or(0);
            let min = t.get(r, "VersionMin").str().map(product_version);
            let max = t.get(r, "VersionMax").str().map(product_version);
            for code in records::related(&mut *self.target, &upgrade) {
                if code.eq_ignore_ascii_case(&own) {
                    continue;
                }
                let Some(record) = records::product(&mut *self.target, &code) else {
                    continue;
                };
                let v = product_version(&record.version);
                let above = min.is_none_or(|m| {
                    if attributes & 0x100 != 0 {
                        v >= m
                    } else {
                        v > m
                    }
                });
                let below = max.is_none_or(|m| {
                    if attributes & 0x200 != 0 {
                        v <= m
                    } else {
                        v < m
                    }
                });
                if !(above && below) {
                    continue;
                }
                let mut list = String::from(self.property(&property));
                if !list.split(';').any(|c| c.eq_ignore_ascii_case(&code)) {
                    if !list.is_empty() {
                        list.push(';');
                    }
                    list.push_str(&code);
                }
                self.log(&format!(
                    "FindRelatedProducts: found related product {code} ({property})."
                ));
                self.set_property(&property, &list);
            }
        }
    }

    /// The plan of the target state and, for an installed product, of what
    /// it has; the components to install and to remove; the space needed.
    pub fn cost_finalize(&mut self) -> Result<(), u32> {
        let machine = self.state.machine.clone().unwrap_or_else(|| Machine {
            properties: Vec::new(),
        });
        let removing = self.state.removing_all();
        let installed_features = self.state.record.as_ref().map(|r| r.features.clone());
        // What is installed: the recorded features with the recorded
        // directory choices.
        let installed_plan = match &installed_features {
            Some(features) => {
                let mut props: Vec<(String, String)> = self
                    .state
                    .properties
                    .iter()
                    .filter(|(k, _)| k != "ADDLOCAL" && k != "REMOVE")
                    .cloned()
                    .collect();
                if let Some(r) = &self.state.record {
                    props.extend(r.properties.iter().cloned());
                }
                props.push((String::from("ADDLOCAL"), features.join(",")));
                let plan = install::plan(
                    self.db,
                    &Machine {
                        properties: Vec::new(),
                    },
                    &props,
                )
                .map_err(|_| code::BAD_PACKAGE)?;
                Some(plan)
            }
            None => None,
        };
        // The target state.
        let mut props = self.state.properties.clone();
        if let Some(features) = &installed_features
            && !props.iter().any(|(k, _)| k == "ADDLOCAL")
        {
            // Maintenance keeps the installed features (plus ADDLOCAL,
            // minus REMOVE).
            props.push((String::from("ADDLOCAL"), features.join(",")));
        }
        let plan = if removing {
            Plan {
                properties: self.state.properties.clone(),
                ..installed_plan.clone().unwrap_or_default()
            }
        } else {
            install::plan(self.db, &machine, &props).map_err(|_| code::BAD_PACKAGE)?
        };
        let installed_components: Vec<String> = installed_plan
            .as_ref()
            .map(|p| p.components.clone())
            .unwrap_or_default();
        let reinstall = !self.property("REINSTALL").is_empty();
        let (install_set, remove_set) = if removing {
            (Vec::new(), installed_components.clone())
        } else {
            let install: Vec<String> = plan
                .components
                .iter()
                .filter(|c| reinstall || !installed_components.contains(c))
                .cloned()
                .collect();
            let remove: Vec<String> = installed_components
                .iter()
                .filter(|c| !plan.components.contains(c))
                .cloned()
                .collect();
            (install, remove)
        };
        // Directory properties ([INSTALLFOLDER]).
        if !removing {
            for (k, v) in &plan.directories {
                self.state.set_property(k, v);
            }
        } else if let Some(p) = &installed_plan {
            for (k, v) in &p.directories {
                self.state.set_property(k, v);
            }
        }
        // Space on the system drive.
        let required: u64 = plan
            .files
            .iter()
            .filter(|f| install_set.contains(&f.component))
            .map(|f| u64::from(f.size))
            .sum();
        let freed: u64 = installed_plan
            .as_ref()
            .map(|p| {
                p.files
                    .iter()
                    .filter(|f| remove_set.contains(&f.component))
                    .map(|f| u64::from(f.size))
                    .sum()
            })
            .unwrap_or(0);
        let drive = String::from(self.property("ROOTDRIVE"));
        let drive = drive.trim_end_matches('\\');
        let need = required.saturating_sub(freed);
        let available = self
            .target
            .free_bytes(if drive.is_empty() { "C:" } else { drive });
        self.state.set_property(
            "PrimaryVolumePath",
            if drive.is_empty() { "C:" } else { drive },
        );
        self.state.set_property(
            "PrimaryVolumeSpaceRequired",
            &format!("{}", need.div_ceil(1024)),
        );
        if let Some(a) = available {
            self.state
                .set_property("PrimaryVolumeSpaceAvailable", &format!("{}", a / 1024));
            self.state.set_property(
                "PrimaryVolumeSpaceRemaining",
                &format!("{}", a.saturating_sub(need) / 1024),
            );
            self.state
                .set_property("OutOfDiskSpace", if need > a { "1" } else { "" });
            self.state
                .set_property("OutOfNoRbDiskSpace", if need > a { "1" } else { "" });
        } else {
            self.state.set_property("OutOfDiskSpace", "");
        }
        self.state.plan = plan;
        self.state.installed_plan = installed_plan;
        self.state.install_set = install_set;
        self.state.remove_set = remove_set;
        self.state.costed = true;
        Ok(())
    }
    /// Costing again after the UI changed a folder or the features.
    pub fn recost(&mut self) {
        let _ = self.cost_finalize();
    }

    fn ensure_costed(&mut self) -> Result<(), u32> {
        if !self.state.costed {
            self.cost_finalize()?;
        }
        Ok(())
    }

    fn install_validate(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        loop {
            if self.property("OutOfDiskSpace") != "1" {
                break;
            }
            let text = format!(
                "Out of disk space -- Volume: '{}'; required space: {} KB; available space: {} KB. Free some disk space and retry.",
                self.property("PrimaryVolumePath"),
                self.property("PrimaryVolumeSpaceRequired"),
                self.property("PrimaryVolumeSpaceAvailable"),
            );
            self.log(&text);
            match ui.message(
                msg::OUT_OF_DISK_SPACE | msg::MB_RETRYCANCEL | msg::MB_ICONERROR,
                &text,
            ) {
                msg::IDRETRY => self.recost(),
                msg::IDIGNORE => break,
                msg::IDCANCEL => return Err(code::INSTALL_USEREXIT),
                _ => return Err(code::INSTALL_FAILURE),
            }
        }
        // Progress: bytes to copy plus a share per registry value,
        // shortcut and component.
        let files: u64 = self
            .state
            .plan
            .files
            .iter()
            .filter(|f| self.state.install_set.contains(&f.component))
            .map(|f| u64::from(f.size))
            .sum();
        let others = (self.state.plan.registry.len()
            + self.state.plan.shortcuts.len()
            + self.state.install_set.len()
            + self.state.remove_set.len()) as u64;
        self.state.total = files + others * 4096 + 65536;
        self.state.done = 0;
        if !ui.progress(0, self.state.total) {
            return Err(code::INSTALL_USEREXIT);
        }
        Ok(())
    }
    fn step(&mut self, ui: &mut dyn Ui, amount: u64) -> Result<(), u32> {
        self.state.done = (self.state.done + amount).min(self.state.total.max(1));
        if ui.progress(self.state.done, self.state.total) {
            Ok(())
        } else {
            self.state.cancelled = true;
            Err(code::INSTALL_USEREXIT)
        }
    }

    // ---- the script --------------------------------------------------------

    fn install_initialize(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        // What Doors' installer does not apply, said in the log: the
        // tables' own permissions (objects keep their folder's or key's
        // inherited ones) and a service account other than LocalSystem
        // (every service runs as LocalSystem, ADR 0026).
        for table in ["LockPermissions", "MsiLockPermissionsEx"] {
            if opt_table(self.db, table).is_some_and(|t| !t.rows.is_empty()) {
                self.log(&format!(
                    "The {table} table is not applied by Doors Installer: the objects keep their inherited permissions."
                ));
            }
        }
        if let Some(t) = opt_table(self.db, "ServiceInstall") {
            for r in 0..t.rows.len() {
                let account = t.get(r, "StartName").str().unwrap_or("");
                if !account.is_empty() && !account.eq_ignore_ascii_case("LocalSystem") {
                    let name = String::from(t.get(r, "Name").str().unwrap_or(""));
                    self.log(&format!(
                        "Service {name} asks for the account {account}; Doors runs every service as LocalSystem."
                    ));
                }
            }
        }
        if self.state.script || self.state.nested {
            self.state.script = true;
            return Ok(());
        }
        let product = String::from(self.state.product_code());
        let journal = Journal::begin(&mut *self.target, &product, self.state.serial)?;
        let boot = format!("{}", self.target.boot());
        for (name, value) in [
            ("Product", product.as_str()),
            ("Script", journal.path.as_str()),
            ("Boot", boot.as_str()),
            (
                "Action",
                if self.state.removing_all() {
                    "Remove"
                } else {
                    "Install"
                },
            ),
        ] {
            self.target.set_value(
                Root::LocalMachine,
                IN_PROGRESS,
                Some(name),
                &RegValue::String(String::from(value)),
            )?;
        }
        self.state.journal = journal;
        self.state.script = true;
        Ok(())
    }

    fn install_finalize(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        if !self.state.script {
            return Ok(());
        }
        if self.state.nested {
            return Ok(());
        }
        // Commit custom actions; their failure does not undo anything.
        let commits = core::mem::take(&mut self.state.commit);
        for (ca, data) in commits {
            self.log(&format!("Commit custom action {}.", ca.name));
            let plain = CustomAction {
                kind: ca.kind & !0x600,
                ..ca
            };
            if let Err(e) = self.run_custom(ui, &plain, Some(data)) {
                self.log(&format!(
                    "Commit custom action {} returned {e}.",
                    plain.name
                ));
            }
        }
        let mut journal = core::mem::take(&mut self.state.journal);
        journal.commit(&mut *self.target);
        self.target.delete_key(Root::LocalMachine, IN_PROGRESS);
        self.state.script = false;
        let _ = self.step(ui, 0);
        Ok(())
    }

    // ---- components --------------------------------------------------------

    fn component(&self, key: &str) -> Option<ComponentInfo> {
        self.state
            .plan
            .component_info
            .iter()
            .chain(
                self.state
                    .installed_plan
                    .iter()
                    .flat_map(|p| p.component_info.iter()),
            )
            .find(|c| c.key == key)
            .cloned()
    }
    /// A component's key path as Windows records it: the file, `02:\key\name`
    /// for a registry key path, or the directory.
    fn keypath(&self, c: &ComponentInfo) -> String {
        let plan = if self
            .state
            .plan
            .component_info
            .iter()
            .any(|x| x.key == c.key && x.selected)
        {
            &self.state.plan
        } else {
            self.state
                .installed_plan
                .as_ref()
                .unwrap_or(&self.state.plan)
        };
        if c.attributes & component_attributes::REGISTRY_KEY_PATH != 0 {
            if let Some(r) = opt_table(self.db, "Registry")
                && let Some(row) = r.find(&c.keypath)
            {
                // 00-03 in the 32-bit view, 20-23 in the 64-bit one; -1 is
                // the installation's (HKLM per machine, HKCU per user).
                let all_users = plan.all_users;
                let root = match r.get(row, "Root").int().unwrap_or(2) {
                    0 => 0,
                    1 => 1,
                    3 => 3,
                    -1 if !all_users => 1,
                    _ => 2,
                } + if plan.component_is_32bit(&c.key) {
                    0
                } else {
                    20
                };
                let key = self.format(r.get(row, "Key").str().unwrap_or(""));
                let name = self.format(r.get(row, "Name").str().unwrap_or(""));
                return format!("{root:02}:\\{key}\\{name}");
            }
            return String::new();
        }
        if !c.keypath.is_empty()
            && let Some(f) = plan.files.iter().find(|f| f.key == c.keypath)
        {
            return f.target.clone();
        }
        c.directory.clone()
    }

    /// ProcessComponents: register this product as a client of the
    /// components it installs; unregister it from those it removes. A
    /// component other products still use (or a permanent one) keeps its
    /// resources.
    fn process_components(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let product = packed_guid(self.state.product_code()).ok_or(code::BAD_PACKAGE)?;
        let install = self.state.install_set.clone();
        let remove = self.state.remove_set.clone();
        for c in &install {
            let Some(info) = self.component(c) else {
                continue;
            };
            if info.guid.is_empty() {
                continue;
            }
            let Some((root, key)) = records::component_key(&info.guid) else {
                continue;
            };
            let path = self.keypath(&info);
            self.j_set_value(root, &key, Some(&product), &RegValue::String(path.clone()))?;
            if info.attributes & component_attributes::PERMANENT != 0 {
                self.j_set_value(
                    root,
                    &key,
                    Some(records::PERMANENT_CLIENT),
                    &RegValue::String(path.clone()),
                )?;
            }
            if info.attributes & component_attributes::SHARED_DLL_REF_COUNT != 0 && !path.is_empty()
            {
                let count = match self.target.value(
                    Root::LocalMachine,
                    records::SHARED_DLLS,
                    &records::shared_dll_name(&path),
                ) {
                    Some(RegValue::Dword(d)) => d,
                    _ => 0,
                };
                // A reinstall does not count again.
                let already = self
                    .state
                    .installed_plan
                    .as_ref()
                    .is_some_and(|p| p.components.contains(c));
                if !already {
                    self.j_set_value(
                        Root::LocalMachine,
                        records::SHARED_DLLS,
                        Some(&records::shared_dll_name(&path)),
                        &RegValue::Dword(count + 1),
                    )?;
                }
            }
        }
        let mut shared = Vec::new();
        for c in &remove {
            let Some(info) = self.component(c) else {
                continue;
            };
            let path = self.keypath(&info);
            let mut keep = info.attributes & component_attributes::PERMANENT != 0;
            if info.attributes & component_attributes::SHARED_DLL_REF_COUNT != 0
                && !path.is_empty()
                && let Some(RegValue::Dword(d)) = self.target.value(
                    Root::LocalMachine,
                    records::SHARED_DLLS,
                    &records::shared_dll_name(&path),
                )
            {
                if d > 1 {
                    keep = true;
                    self.j_set_value(
                        Root::LocalMachine,
                        records::SHARED_DLLS,
                        Some(&records::shared_dll_name(&path)),
                        &RegValue::Dword(d - 1),
                    )?;
                } else {
                    self.j_delete_value(
                        Root::LocalMachine,
                        records::SHARED_DLLS,
                        Some(&records::shared_dll_name(&path)),
                    )?;
                }
            }
            if !info.guid.is_empty()
                && let Some((root, key)) = records::component_key(&info.guid)
            {
                self.j_delete_value(root, &key, Some(&product))?;
                let others = records::clients(&mut *self.target, &info.guid);
                if !others.is_empty() {
                    keep = true;
                    self.log(&format!(
                        "Component {c} is shared with {} other client(s); its resources stay.",
                        others.len()
                    ));
                } else {
                    self.j_delete_key(root, &key)?;
                }
            }
            if keep {
                shared.push(c.clone());
            }
        }
        self.state.shared = shared;
        Ok(())
    }
    /// Components whose resources this session removes.
    fn removing(&self) -> Vec<String> {
        self.state
            .remove_set
            .iter()
            .filter(|c| !self.state.shared.contains(c))
            .cloned()
            .collect()
    }
    fn installed(&self) -> &Plan {
        self.state
            .installed_plan
            .as_ref()
            .unwrap_or(&self.state.plan)
    }

    // ---- services ----------------------------------------------------------

    fn stop_services(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        let controls: Vec<_> = self
            .installed()
            .service_controls
            .iter()
            .chain(self.state.plan.service_controls.iter())
            .cloned()
            .collect();
        let removing = self.removing();
        let mut done: Vec<String> = Vec::new();
        for s in controls {
            let stop = (removing.contains(&s.component)
                && s.event & service_event::UNINSTALL_STOP != 0)
                || (self.state.install_set.contains(&s.component)
                    && s.event & service_event::INSTALL_STOP != 0);
            if stop && !done.contains(&s.name) && self.target.service_exists(&s.name) {
                done.push(s.name.clone());
                self.record(Undo::ServiceChanged(s.name.clone()))?;
                if let Err(e) = self.target.stop_service(&s.name, s.wait) {
                    self.log(&format!("Could not stop service {}: {e}.", s.name));
                }
                self.step(ui, 4096)?;
            }
        }
        Ok(())
    }
    fn delete_services(&mut self) -> Result<(), u32> {
        let controls: Vec<_> = self.installed().service_controls.clone();
        let removing = self.removing();
        for s in controls {
            if removing.contains(&s.component)
                && s.event & service_event::UNINSTALL_DELETE != 0
                && self.target.service_exists(&s.name)
            {
                self.record(Undo::ServiceChanged(s.name.clone()))?;
                let _ = self.target.delete_service(&s.name);
            }
        }
        Ok(())
    }
    fn install_services(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        let services = self.state.plan.services.clone();
        for s in services {
            if !self.state.install_set.contains(&s.component) {
                continue;
            }
            if self.target.service_exists(&s.name) {
                continue;
            }
            self.record(Undo::Service(s.name.clone()))?;
            if let Err(e) = self.target.install_service(&s) {
                self.log(&format!("InstallServices: {} failed with {e}.", s.name));
                let display = s.display_name.clone();
                self.error(ui, 1923, &[&display, &s.name], msg::MB_OK);
                return Err(code::INSTALL_FAILURE);
            }
            self.step(ui, 4096)?;
        }
        Ok(())
    }
    fn start_services(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        let controls = self.state.plan.service_controls.clone();
        for s in controls {
            if !self.state.install_set.contains(&s.component)
                || s.event & service_event::INSTALL_START == 0
            {
                continue;
            }
            self.record(Undo::ServiceChanged(s.name.clone()))?;
            if let Err(e) = self.target.start_service(&s.name, s.wait) {
                self.log(&format!("StartServices: {} failed with {e}.", s.name));
                let display = self
                    .state
                    .plan
                    .services
                    .iter()
                    .find(|x| x.name == s.name)
                    .map(|x| x.display_name.clone())
                    .unwrap_or_else(|| s.name.clone());
                if self.error(ui, 1920, &[&display, &s.name], msg::MB_ABORTRETRYIGNORE)
                    != msg::IDIGNORE
                {
                    return Err(code::INSTALL_FAILURE);
                }
            }
        }
        Ok(())
    }

    // ---- removal -----------------------------------------------------------

    fn unpublish_features(&mut self) -> Result<(), u32> {
        if !self.state.removing_all() || self.state.record.is_none() {
            return Ok(());
        }
        let all_users = self.state.record.as_ref().is_some_and(|r| r.all_users);
        if let Some((root, key)) = records::features_key(all_users, self.state.product_code()) {
            self.j_delete_key(root, &key)?;
        }
        Ok(())
    }
    /// RemoveRegistryValues (with the RemoveRegistry table) and the
    /// unregistration actions of the other registry groups.
    fn remove_registry(&mut self, group: RegistryGroup) -> Result<(), u32> {
        self.ensure_costed()?;
        let removing = self.removing();
        let rows: Vec<_> = self
            .installed()
            .registry
            .iter()
            .filter(|r| r.group == group && removing.contains(&r.component))
            .cloned()
            .collect();
        for r in &rows {
            if r.value.is_some() {
                self.j_delete_value(r.root, &r.key, r.name.as_deref())?;
            }
        }
        // Keys the package marked for removal ("-", "*", associations).
        for r in rows.iter().filter(|r| r.remove_key) {
            if self.key_exists(r.root, &r.key) {
                self.j_delete_key(r.root, &r.key)?;
            }
        }
        // The RemoveRegistry table.
        if group != RegistryGroup::Registry {
            return Ok(());
        }
        if let Some(t) = opt_table(self.db, "RemoveRegistry") {
            for i in 0..t.rows.len() {
                let component = t.get(i, "Component_").str().unwrap_or("");
                if !self.state.install_set.iter().any(|c| c == component) {
                    continue;
                }
                let root = match t.get(i, "Root").int().unwrap_or(2) {
                    0 => Root::ClassesRoot,
                    1 => Root::CurrentUser,
                    3 => Root::Users,
                    -1 if !self.state.all_users() => Root::CurrentUser,
                    _ => Root::LocalMachine,
                }
                .in_view(self.state.plan.component_is_32bit(component));
                let key = self.format(t.get(i, "Key").str().unwrap_or(""));
                match t.get(i, "Name").str() {
                    Some("-") => self.j_delete_key(root, &key)?,
                    Some(n) => {
                        let n = self.format(n);
                        self.j_delete_value(root, &key, Some(&n))?
                    }
                    None => self.j_delete_value(root, &key, None)?,
                }
            }
        }
        Ok(())
    }
    fn remove_shortcuts(&mut self) -> Result<(), u32> {
        let removing = self.removing();
        let paths: Vec<String> = self
            .installed()
            .shortcuts
            .iter()
            .filter(|s| removing.contains(&s.component))
            .map(|s| s.path.clone())
            .collect();
        for p in paths {
            self.j_remove_file(&p)?;
        }
        Ok(())
    }
    fn remove_files(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        let removing = self.removing();
        let files: Vec<(String, u32)> = self
            .installed()
            .files
            .iter()
            .filter(|f| removing.contains(&f.component))
            .map(|f| (f.target.clone(), f.size))
            .collect();
        for (path, size) in files {
            self.j_remove_file(&path)?;
            self.step(ui, u64::from(size) / 4 + 1)?;
        }
        // RemoveFile rows: on removal (2, 3) for removed components, on
        // installation (1, 3) for installed ones.
        let rows: Vec<_> = self
            .installed()
            .remove_files
            .iter()
            .chain(self.state.plan.remove_files.iter())
            .cloned()
            .collect();
        for r in rows {
            let applies = (removing.contains(&r.component) && r.mode & 2 != 0)
                || (self.state.install_set.contains(&r.component) && r.mode & 1 != 0);
            if !applies {
                continue;
            }
            let Some(pattern) = &r.pattern else {
                continue;
            };
            let folder = r.folder.trim_end_matches('\\').to_string();
            for name in self.target.list(&folder) {
                if wildcard(pattern, &name) {
                    self.j_remove_file(&format!("{folder}\\{name}"))?;
                }
            }
        }
        Ok(())
    }
    fn remove_folders(&mut self) -> Result<(), u32> {
        let removing = self.removing();
        if removing.is_empty() {
            return Ok(());
        }
        let plan = self.installed().clone();
        let system = system_folders(&plan);
        let mut folders: Vec<String> = Vec::new();
        for f in &plan.files {
            if removing.contains(&f.component)
                && let Some(i) = f.target.rfind('\\')
            {
                folders.push(String::from(&f.target[..=i]));
            }
        }
        for s in &plan.shortcuts {
            if removing.contains(&s.component)
                && let Some(i) = s.path.rfind('\\')
            {
                folders.push(String::from(&s.path[..=i]));
            }
        }
        for c in &plan.component_info {
            if removing.contains(&c.key) {
                folders.push(c.directory.clone());
            }
        }
        for r in &plan.remove_files {
            if removing.contains(&r.component) && r.pattern.is_none() {
                folders.push(r.folder.clone());
            }
        }
        folders.extend(plan.folders.iter().cloned());
        folders.sort_by_key(|f| core::cmp::Reverse(f.matches('\\').count()));
        folders.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        for f in folders {
            let path = f.trim_end_matches('\\').to_string();
            if path.is_empty()
                || system
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(&format!("{path}\\")))
            {
                continue;
            }
            // Folders still holding the user's files stay.
            if !self.j_remove_folder(&path) {
                continue;
            }
            let mut parent = path.as_str();
            while let Some(i) = parent.rfind('\\') {
                parent = &parent[..i];
                let with = format!("{parent}\\");
                if parent.ends_with(':') || system.iter().any(|s| s.eq_ignore_ascii_case(&with)) {
                    break;
                }
                if !self.j_remove_folder(parent) {
                    break;
                }
            }
        }
        Ok(())
    }

    // ---- installation ------------------------------------------------------

    fn create_folders(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let plan = self.state.plan.clone();
        let mut folders: Vec<String> = Vec::new();
        if let Some(t) = opt_table(self.db, "CreateFolder") {
            for r in 0..t.rows.len() {
                let component = t.get(r, "Component_").str().unwrap_or("");
                if self.state.install_set.iter().any(|c| c == component)
                    && let Some(d) = plan.directory(t.get(r, "Directory_").str().unwrap_or(""))
                {
                    folders.push(String::from(d));
                }
            }
        }
        for f in folders {
            self.j_folder_tree(&f)?;
        }
        Ok(())
    }

    /// Whether `file` replaces the file already at its target, by Windows
    /// Installer's file versioning rules and REINSTALLMODE.
    fn should_copy(
        &mut self,
        file: &install::FileCopy,
        info: &Option<ComponentInfo>,
    ) -> (bool, &'static str) {
        if !self.target.exists(&file.target) {
            return (true, "");
        }
        if info
            .as_ref()
            .is_some_and(|c| c.attributes & component_attributes::NEVER_OVERWRITE != 0)
        {
            return (
                false,
                "Won't Overwrite; Won't patch; Existing file is a never-overwrite component's",
            );
        }
        let mode = self.property("REINSTALLMODE").to_ascii_lowercase();
        if mode.contains('a') {
            return (true, "");
        }
        let new = version::parse(&file.version);
        let old = self
            .target
            .read_file(&file.target)
            .ok()
            .and_then(|b| version::of_image(&b));
        match (new, old) {
            (Some(n), Some(o)) => {
                let copy = if mode.contains('e') {
                    n >= o
                } else if mode.contains('d') {
                    n != o
                } else if mode.contains('o') {
                    n > o
                } else {
                    false
                };
                (
                    copy,
                    if copy {
                        ""
                    } else {
                        "Won't Overwrite; Won't patch; Existing file is of an equal version"
                    },
                )
            }
            (Some(_), None) => (true, ""),
            (None, Some(_)) => (
                false,
                "Won't Overwrite; Won't patch; Existing file is versioned",
            ),
            (None, None) => {
                // An unversioned file the user changed since it was
                // installed stays.
                if let Some((created, modified)) = self.target.file_times(&file.target)
                    && modified > created + 2
                {
                    return (
                        false,
                        "Won't Overwrite; Won't patch; Existing file is unversioned but modified",
                    );
                }
                let missing_only = mode.contains('p') && !mode.contains('o') && !mode.contains('e');
                (
                    !missing_only,
                    if missing_only {
                        "Won't Overwrite; Existing file present"
                    } else {
                        ""
                    },
                )
            }
        }
    }

    fn install_files(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        let package_dir = {
            let p = &self.state.package;
            p.rfind('\\')
                .map_or(String::new(), |i| String::from(&p[..=i]))
        };
        let files: Vec<install::FileCopy> = self
            .state
            .plan
            .files
            .iter()
            .filter(|f| self.state.install_set.contains(&f.component))
            .cloned()
            .collect();
        let mut opened: Vec<(Cabinet, Vec<u8>)> = Vec::new();
        for file in files {
            let info = self.component(&file.component);
            let (copy, reason) = self.should_copy(&file, &info);
            if !copy {
                self.log(&format!("{reason}: {}", file.target));
                self.step(ui, u64::from(file.size))?;
                continue;
            }
            ui.message(
                msg::ACTION_DATA,
                &format!(
                    "File: {}, Directory: {}, Size: {}",
                    file.target.rsplit('\\').next().unwrap_or(""),
                    file.target.rfind('\\').map_or("", |i| &file.target[..=i]),
                    file.size
                ),
            );
            let data = loop {
                let result = (|| -> Result<Vec<u8>, (u32, String)> {
                    match &file.source {
                        Source::Cabinet { cabinet, name } => {
                            if !opened.iter().any(|(c, _)| c == cabinet) {
                                let bytes = match cabinet {
                                    Cabinet::Embedded(stream) => self
                                        .db
                                        .stream(stream)
                                        .map_err(|_| (1335, stream.clone()))?,
                                    Cabinet::External(f) => self
                                        .target
                                        .read_file(&format!("{package_dir}{f}"))
                                        .map_err(|_| (1311, format!("{package_dir}{f}")))?,
                                };
                                opened.push((cabinet.clone(), bytes));
                            }
                            let bytes = &opened.iter().find(|(c, _)| c == cabinet).unwrap().1;
                            let label = match cabinet {
                                Cabinet::Embedded(s) | Cabinet::External(s) => s.clone(),
                            };
                            let cab = door_cab::Cabinet::parse(bytes)
                                .map_err(|_| (1335, label.clone()))?;
                            let entry = cab.file(name).ok_or((1335, label.clone()))?;
                            cab.extract(entry).map_err(|_| (1335, label))
                        }
                        Source::Loose(name) => self
                            .target
                            .read_file(&format!("{package_dir}{name}"))
                            .map_err(|_| (1311, format!("{package_dir}{name}"))),
                        Source::Empty => Ok(Vec::new()),
                    }
                })();
                match result {
                    Ok(d) => break d,
                    Err((number, what)) => {
                        let answer = self.error(ui, number, &[&what], msg::MB_RETRYCANCEL);
                        if answer != msg::IDRETRY {
                            return Err(code::INSTALL_FAILURE);
                        }
                    }
                }
            };
            self.log(&format!("File: {}", file.target));
            loop {
                match self.j_write_file(&file.target, &data) {
                    Ok(()) => break,
                    Err(e) => {
                        self.log(&format!(
                            "Writing {} failed: system error {e}.",
                            file.target
                        ));
                        let target = file.target.clone();
                        let answer = self.error(ui, 1310, &[&target], msg::MB_RETRYCANCEL);
                        if answer != msg::IDRETRY {
                            return Err(if answer == msg::IDCANCEL {
                                code::INSTALL_USEREXIT
                            } else {
                                code::INSTALL_FAILURE
                            });
                        }
                    }
                }
            }
            self.step(ui, u64::from(file.size))?;
        }
        Ok(())
    }

    /// WriteRegistryValues and the registration actions whose tables are
    /// registry rows (`group`): RegisterExtensionInfo, RegisterProgIdInfo,
    /// RegisterClassInfo, RegisterMIMEInfo, PublishComponents.
    fn write_registry(&mut self, ui: &mut dyn Ui, group: RegistryGroup) -> Result<(), u32> {
        self.ensure_costed()?;
        // Windows formats the values here, with what custom actions set
        // since costing (a DLL action's property, say).
        if !self.state.removing_all() {
            let machine = self.state.machine.clone().unwrap_or_else(|| Machine {
                properties: Vec::new(),
            });
            let mut props = self.state.properties.clone();
            if let Some(record) = &self.state.record
                && !props.iter().any(|(k, _)| k == "ADDLOCAL")
            {
                props.push((String::from("ADDLOCAL"), record.features.join(",")));
            }
            if let Ok(fresh) = install::plan(self.db, &machine, &props) {
                self.state.plan.registry = fresh.registry;
            }
        }
        let reinstall = !self.property("REINSTALL").is_empty();
        let mode = self.property("REINSTALLMODE").to_ascii_lowercase();
        let rows: Vec<_> = self
            .state
            .plan
            .registry
            .iter()
            .filter(|r| r.group == group && self.state.install_set.contains(&r.component))
            .cloned()
            .collect();
        for r in rows {
            // A repair rewrites the machine's values with 'm' and the
            // user's with 'u'.
            if reinstall
                && self
                    .state
                    .installed_plan
                    .as_ref()
                    .is_some_and(|p| p.components.contains(&r.component))
            {
                let user = r.root.native() == Root::CurrentUser;
                if (user && !mode.contains('u')) || (!user && !mode.contains('m')) {
                    continue;
                }
            }
            match &r.value {
                Some(v) => {
                    if let Err(e) = self.j_set_value(r.root, &r.key, r.name.as_deref(), v) {
                        self.log(&format!(
                            "Writing {}\\{} failed: {e}.",
                            r.root.name(),
                            r.key
                        ));
                        let name = r.name.clone().unwrap_or_default();
                        let key = format!("{}\\{}", r.root.name(), r.key);
                        self.error(ui, 1406, &[&name, &key], msg::MB_OK);
                        return Err(code::INSTALL_FAILURE);
                    }
                }
                None => {
                    if !self.key_exists(r.root, &r.key) {
                        self.record(Undo::Key {
                            root: r.root,
                            key: r.key.clone(),
                        })?;
                        self.target.create_key(r.root, &r.key)?;
                    }
                }
            }
            self.step(ui, 4096)?;
        }
        Ok(())
    }

    fn create_shortcuts(&mut self) -> Result<(), u32> {
        let reinstall = !self.property("REINSTALL").is_empty();
        let mode = self.property("REINSTALLMODE").to_ascii_lowercase();
        let shortcuts: Vec<_> = self
            .state
            .plan
            .shortcuts
            .iter()
            .filter(|s| self.state.install_set.contains(&s.component))
            .cloned()
            .collect();
        for s in shortcuts {
            if reinstall && !mode.contains('s') && self.target.exists(&s.path) {
                continue;
            }
            let link = Link {
                target: s.target.clone(),
                arguments: s.arguments.clone(),
                description: s.description.clone(),
                working_directory: s.working_directory.clone(),
                show: s.show.max(1) as u32,
            };
            self.j_write_file(&s.path, &link.encode())?;
        }
        Ok(())
    }

    // ---- registration ------------------------------------------------------

    /// RegisterProduct: the cached package, the Installer's record of the
    /// product and Add/Remove Programs; on removal, all of that goes.
    fn register_product(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let product = String::from(self.state.product_code());
        let all_users = self.state.all_users();
        if self.state.removing_all() {
            let record = self.state.record.clone();
            let all_users = record.as_ref().is_some_and(|r| r.all_users);
            if let Some((root, key)) = records::product_key(all_users, &product) {
                self.j_delete_key(root, &key)?;
            }
            let arp = self.installed().uninstall_key.clone();
            let root = self.installed().uninstall_root;
            if !arp.is_empty() {
                let key = String::from(split_root(&arp).1);
                self.j_delete_key(root, &key)?;
            }
            if let Some(r) = record {
                if !r.local_package.is_empty() {
                    self.j_remove_file(&r.local_package)?;
                }
                if !r.upgrade_code.is_empty()
                    && let Some((root, key)) = records::upgrade_key(all_users, &r.upgrade_code)
                {
                    let packed = packed_guid(&product).unwrap_or_default();
                    self.j_delete_value(root, &key, Some(&packed))?;
                    if self.target.values(root, &key).is_empty() {
                        self.j_delete_key(root, &key)?;
                    }
                }
            }
            return Ok(());
        }
        // Cache the package (a repair with 'v' recaches it).
        let cache = match &self.state.record {
            Some(r)
                if !r.local_package.is_empty()
                    && self.target.exists(&r.local_package)
                    && !self.property("REINSTALLMODE").contains('v')
                    && r.package_code
                        .eq_ignore_ascii_case(self.property("PackageCode")) =>
            {
                r.local_package.clone()
            }
            Some(r) if !r.local_package.is_empty() => r.local_package.clone(),
            _ => format!("C:\\Windows\\Installer\\{:x}.msi", self.state.serial),
        };
        if !cache.eq_ignore_ascii_case(&self.state.package) {
            let bytes = self.target.read_file(&self.state.package)?;
            let unchanged = self.target.read_file(&cache).is_ok_and(|old| old == bytes);
            if !unchanged {
                self.j_write_file(&cache, &bytes)?;
            }
        }
        let (root, key) = records::product_key(all_users, &product).ok_or(code::BAD_PACKAGE)?;
        let s = |v: &str| RegValue::String(String::from(v));
        let package_dir = {
            let p = &self.state.package;
            String::from(p.rfind('\\').map_or("", |i| &p[..=i]))
        };
        let today = self.target.today().unwrap_or_default();
        let date = self
            .state
            .record
            .as_ref()
            .map(|r| r.install_date.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or(today);
        // What a later session needs to find the same folders.
        let mut remembered: Vec<String> = self
            .state
            .plan
            .directories
            .iter()
            .filter(|(k, _)| k.chars().all(|c| !c.is_ascii_lowercase()) && k.len() > 1)
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        remembered.push(format!("ALLUSERS={}", self.property("ALLUSERS")));
        let version = String::from(self.property("ProductVersion"));
        let packed_version = {
            let v = version::parse(&version).unwrap_or(0);
            ((v >> 48) as u32) << 24
                | (((v >> 32) & 0xff) as u32) << 16
                | ((v >> 16) & 0xffff) as u32
        };
        let values = [
            ("ProductName", s(self.property("ProductName"))),
            ("VersionString", s(&version)),
            ("Version", RegValue::Dword(packed_version)),
            ("Publisher", s(self.property("Manufacturer"))),
            ("PackageCode", s(self.property("PackageCode"))),
            ("UpgradeCode", s(self.property("UpgradeCode"))),
            ("LocalPackage", s(&cache)),
            ("InstallSource", s(&package_dir)),
            ("InstallDate", s(&date)),
            ("Language", s(self.property("ProductLanguage"))),
            ("Assignment", RegValue::Dword(u32::from(all_users))),
            ("Properties", RegValue::Multi(remembered)),
        ];
        for (name, value) in values {
            self.j_set_value(root, &key, Some(name), &value)?;
        }
        // Add/Remove Programs.
        let arp = self.state.plan.uninstall_key.clone();
        let aroot = self.state.plan.uninstall_root;
        let akey = String::from(split_root(&arp).1);
        let mut entries = self.state.plan.uninstall.clone();
        entries.push((String::from("InstallSource"), s(&package_dir)));
        if !date.is_empty() {
            entries.push((String::from("InstallDate"), s(&date)));
        }
        for (name, value) in entries {
            self.j_set_value(aroot, &akey, Some(&name), &value)?;
        }
        Ok(())
    }
    fn publish_features(&mut self) -> Result<(), u32> {
        if self.state.removing_all() {
            return Ok(());
        }
        let all_users = self.state.all_users();
        let (root, key) =
            records::features_key(all_users, self.state.product_code()).ok_or(code::BAD_PACKAGE)?;
        let features = self.state.plan.features.clone();
        let old: Vec<String> = self
            .target
            .values(root, &key)
            .into_iter()
            .map(|v| v.0)
            .collect();
        for f in old.iter().filter(|f| !features.contains(f)) {
            self.j_delete_value(root, &key, Some(f))?;
        }
        for f in features {
            if !old.contains(&f) {
                self.j_set_value(root, &key, Some(&f), &RegValue::String(String::new()))?;
            }
        }
        Ok(())
    }
    fn publish_product(&mut self) -> Result<(), u32> {
        if self.state.removing_all() {
            return Ok(());
        }
        let upgrade = String::from(self.property("UpgradeCode"));
        if upgrade.is_empty() {
            return Ok(());
        }
        let all_users = self.state.all_users();
        let (root, key) = records::upgrade_key(all_users, &upgrade).ok_or(code::BAD_PACKAGE)?;
        let packed = packed_guid(self.state.product_code()).ok_or(code::BAD_PACKAGE)?;
        self.j_set_value(root, &key, Some(&packed), &RegValue::String(String::new()))
    }

    // ---- upgrades ----------------------------------------------------------

    /// RemoveExistingProducts: remove the products FindRelatedProducts
    /// found (rows without OnlyDetect), each from its cached package. Inside
    /// the script the removal shares this installation's rollback.
    fn remove_existing(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        let Some(t) = opt_table(self.db, "Upgrade") else {
            return Ok(());
        };
        let own = String::from(self.state.product_code());
        let mut codes: Vec<String> = Vec::new();
        for r in 0..t.rows.len() {
            if t.get(r, "Attributes").int().unwrap_or(0) & 0x2 != 0 {
                continue;
            }
            let property = t.get(r, "ActionProperty").str().unwrap_or("");
            for code in self.property(property).split(';').filter(|c| !c.is_empty()) {
                if !codes.iter().any(|c| c.eq_ignore_ascii_case(code)) {
                    codes.push(String::from(code));
                }
            }
        }
        for code in codes {
            let Some(record) = records::product(&mut *self.target, &code) else {
                continue;
            };
            let bytes = match self.target.read_file(&record.local_package) {
                Ok(b) => b,
                Err(_) => {
                    self.log(&format!(
                        "RemoveExistingProducts: the cached package of {code} is missing."
                    ));
                    return Err(code::INSTALL_SOURCE_ABSENT);
                }
            };
            let Ok(db) = Database::open(&bytes) else {
                return Err(code::BAD_PACKAGE);
            };
            self.log(&format!(
                "RemoveExistingProducts: removing {code} ({}).",
                record.name
            ));
            let machine = self.state.machine.clone().unwrap_or(Machine {
                properties: Vec::new(),
            });
            let shared_journal = self.state.script;
            let mut nested = Session::new(
                &db,
                &mut *self.target,
                &record.local_package,
                Request::Remove,
                machine,
                &[(String::from("UPGRADINGPRODUCTCODE"), own.clone())],
                level::NONE,
            );
            nested.state.nested = true;
            if shared_journal {
                nested.state.journal = core::mem::take(&mut self.state.journal);
                nested.state.script = true;
            }
            let result = nested.sequence(ui, "InstallExecuteSequence");
            let journal = core::mem::take(&mut nested.state.journal);
            let log = core::mem::take(&mut nested.state.log);
            if shared_journal {
                self.state.journal = journal;
            } else if result.is_ok() {
                let mut j = journal;
                j.commit(&mut *self.target);
            }
            self.state.log.extend(log);
            match result {
                Ok(()) | Err(code::NO_MORE_ITEMS) => (),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    // ---- custom actions ----------------------------------------------------

    fn custom(&mut self, ui: &mut dyn Ui, ca: &CustomAction) -> Result<(), u32> {
        if ca.kind & 0x400 != 0 {
            if !self.state.script {
                self.error(ui, 2762, &[&ca.name], msg::MB_OK);
                return Err(code::INSTALL_FAILURE);
            }
            let data = String::from(self.property(&ca.name));
            if ca.kind & 0x100 != 0 && ca.kind & 0x200 == 0 {
                // A program's command line is resolved now, as the script
                // records it: rollback (even after a crash, without the
                // package) runs exactly that.
                let (kind, source, target) = match self.resolve_program(ca, &data) {
                    Some((line, directory)) => ((ca.kind & !0x3f) | 34, directory, line),
                    None => (ca.kind, ca.source.clone(), ca.target.clone()),
                };
                return self.record(Undo::Action {
                    name: ca.name.clone(),
                    kind,
                    source,
                    target,
                    data,
                });
            }
            if ca.kind & 0x200 != 0 {
                self.state.commit.push((ca.clone(), data));
                return Ok(());
            }
            // Deferred: it changes the system outside the script. Record
            // that before it runs, so even an interrupted run is known.
            if matches!(ca.kind & 0x3f, 1 | 2 | 17 | 18 | 34 | 50) {
                self.record(Undo::Nontransactional(ca.name.clone()))?;
            }
            return self.run_custom(ui, ca, Some(data));
        }
        self.run_custom(ui, ca, None)
    }

    /// The command line and folder of an EXE custom action from an
    /// installed file (18), a directory (34) or a property (50), with
    /// `data` as CustomActionData.
    fn resolve_program(&mut self, ca: &CustomAction, data: &str) -> Option<(String, String)> {
        let old = String::from(self.property("CustomActionData"));
        self.state.set_property("CustomActionData", data);
        let args = self.format(&ca.target);
        self.state.set_property("CustomActionData", &old);
        match ca.kind & 0x3f {
            18 => {
                let exe = self
                    .state
                    .plan
                    .files
                    .iter()
                    .find(|f| f.key == ca.source)?
                    .target
                    .clone();
                let dir = exe
                    .rfind('\\')
                    .map_or(String::new(), |i| String::from(&exe[..i]));
                Some((format!("\"{exe}\" {args}").trim_end().to_string(), dir))
            }
            34 => {
                let dir = self.state.plan.directory(&ca.source).map(String::from)?;
                Some((args, dir))
            }
            50 => {
                let exe = String::from(self.property(&ca.source));
                Some((
                    format!("\"{exe}\" {args}").trim_end().to_string(),
                    String::new(),
                ))
            }
            _ => None,
        }
    }

    fn run_custom(
        &mut self,
        ui: &mut dyn Ui,
        ca: &CustomAction,
        data: Option<String>,
    ) -> Result<(), u32> {
        let base = ca.kind & 0x3f;
        let ignore_exit = ca.kind & 0x40 != 0;
        let wait = ca.kind & 0x80 == 0;
        if let Some(d) = &data {
            self.state.set_property("CustomActionData", d);
        }
        let result = match base {
            51 => {
                let v = self.format(&ca.target);
                self.log(&format!("PROPERTY CHANGE: {} = {v}", ca.source));
                self.set_property(&ca.source, &v);
                Ok(())
            }
            35 => {
                let mut v = self.format(&ca.target);
                if !v.is_empty() && !v.ends_with('\\') {
                    v.push('\\');
                }
                self.set_property(&ca.source, &v);
                if self.state.costed {
                    self.recost();
                }
                Ok(())
            }
            19 => {
                let text = match ca.target.parse::<u32>() {
                    Ok(n) => self.error_message(n, &[]),
                    Err(_) => self.format(&ca.target),
                };
                self.log(&text);
                ui.message(msg::ERROR | msg::MB_OK | msg::MB_ICONERROR, &text);
                Err(code::INSTALL_FAILURE)
            }
            1 | 17 => self.dll_action(ui, ca, base),
            2 | 18 | 34 | 50 => self.exe_action(ui, ca, base, wait, ignore_exit),
            5 | 21 | 37 | 53 => {
                self.error(ui, 2739, &[&ca.name], msg::MB_OK);
                Err(code::INSTALL_FAILURE)
            }
            6 | 22 | 38 | 54 => {
                self.error(ui, 2738, &[&ca.name], msg::MB_OK);
                Err(code::INSTALL_FAILURE)
            }
            _ => {
                self.log(&format!(
                    "Custom action {} of type {} is not supported.",
                    ca.name, ca.kind
                ));
                Err(code::INSTALL_FAILURE)
            }
        };
        if data.is_some() {
            self.state.set_property("CustomActionData", "");
        }
        result
    }

    /// A Binary table stream written to a temporary file for a custom
    /// action.
    fn binary_file(&mut self, key: &str, extension: &str) -> Result<String, u32> {
        let bytes = self
            .db
            .stream(&format!("Binary.{key}"))
            .map_err(|_| code::BAD_PACKAGE)?;
        let folder = String::from(self.property("TempFolder"));
        let folder = if folder.is_empty() {
            String::from("C:\\Windows\\Installer\\")
        } else {
            folder
        };
        let path = format!(
            "{folder}MSI{:X}{extension}",
            (self.state.serial as u32) ^ key.len() as u32 ^ (self.state.log.len() as u32) << 8
        );
        if let Some(i) = path.rfind('\\') {
            let parent = String::from(&path[..i]);
            if !self.target.exists(&parent) {
                let _ = self.target.create_folder(&parent);
            }
        }
        self.target.write_file(&path, &bytes)?;
        Ok(path)
    }

    fn dll_action(&mut self, ui: &mut dyn Ui, ca: &CustomAction, base: i32) -> Result<(), u32> {
        let (path, temporary) = if base == 1 {
            (self.binary_file(&ca.source, ".tmp")?, true)
        } else {
            let file = self
                .state
                .plan
                .files
                .iter()
                .find(|f| f.key == ca.source)
                .map(|f| f.target.clone());
            match file {
                Some(p) => (p, false),
                None => return Err(code::BAD_PACKAGE),
            }
        };
        self.log(&format!(
            "Invoking custom action {} from {path} (entry {}).",
            ca.name, ca.target
        ));
        let result = self.target.call_dll(&path, &ca.target, &mut self.state);
        if temporary {
            let _ = self.target.remove_file(&path);
        }
        match result {
            Ok(0) => Ok(()),
            Ok(code::NO_MORE_ITEMS) => Err(code::NO_MORE_ITEMS),
            Ok(_) if ca.kind & 0x40 != 0 => Ok(()),
            Ok(code::INSTALL_USEREXIT) => Err(code::INSTALL_USEREXIT),
            Ok(r) => {
                self.log(&format!(
                    "Custom action {} returned actual error code {r}.",
                    ca.name
                ));
                Err(code::INSTALL_FAILURE)
            }
            Err(e) => {
                self.log(&format!(
                    "Custom action {}: the DLL could not be run ({e}).",
                    ca.name
                ));
                // ERROR_BAD_EXE_FORMAT: a DLL of the other bitness. A
                // package's installation runs at the package's bitness;
                // Doors has no custom action server for the other one.
                if e == 193 {
                    self.log(&format!(
                        "Custom action {}: its DLL is not of this package's platform; Doors Installer runs DLL custom actions at the package's bitness only.",
                        ca.name
                    ));
                }
                self.error(ui, 1723, &[&ca.name, &ca.target, &path], msg::MB_OK);
                Err(code::INSTALL_FAILURE)
            }
        }
    }

    fn exe_action(
        &mut self,
        ui: &mut dyn Ui,
        ca: &CustomAction,
        base: i32,
        wait: bool,
        ignore_exit: bool,
    ) -> Result<(), u32> {
        let args = self.format(&ca.target);
        let (line, directory, temporary) = match base {
            2 => {
                let exe = self.binary_file(&ca.source, ".exe")?;
                (
                    format!("\"{exe}\" {args}"),
                    String::from(self.property("TempFolder")),
                    Some(exe),
                )
            }
            18 => {
                let exe = self
                    .state
                    .plan
                    .files
                    .iter()
                    .find(|f| f.key == ca.source)
                    .map(|f| f.target.clone())
                    .ok_or(code::BAD_PACKAGE)?;
                let dir = exe
                    .rfind('\\')
                    .map_or(String::new(), |i| String::from(&exe[..i]));
                (format!("\"{exe}\" {args}"), dir, None)
            }
            34 => {
                // A literal folder (a resolved rollback action's) or a
                // Directory key.
                let dir = if ca.source.contains(":\\") || ca.source.is_empty() {
                    ca.source.clone()
                } else {
                    self.state
                        .plan
                        .directory(&ca.source)
                        .map(String::from)
                        .unwrap_or_else(|| String::from(self.property(&ca.source)))
                };
                (args, dir, None)
            }
            _ => {
                let exe = String::from(self.property(&ca.source));
                (format!("\"{exe}\" {args}"), String::new(), None)
            }
        };
        let line = line.trim_end().to_string();
        self.log(&format!("Custom action {} runs: {line}", ca.name));
        let result = self
            .target
            .run(&line, directory.trim_end_matches('\\'), wait);
        if let Some(t) = temporary
            && wait
        {
            let _ = self.target.remove_file(&t);
        }
        match result {
            Ok(0) => Ok(()),
            Ok(_) if !wait || ignore_exit => Ok(()),
            Ok(exit) => {
                self.log(&format!(
                    "Custom action {} returned exit code {exit}.",
                    ca.name
                ));
                let location = directory.clone();
                self.error(ui, 1722, &[&ca.name, &location, &line], msg::MB_OK);
                Err(code::INSTALL_FAILURE)
            }
            Err(e) => {
                self.log(&format!(
                    "Custom action {}: the program could not be run ({e}).",
                    ca.name
                ));
                self.error(ui, 1721, &[&ca.name, &directory, &line], msg::MB_OK);
                Err(code::INSTALL_FAILURE)
            }
        }
    }
}

/// `*` and `?` file name matching, case-insensitive.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first() == Some(c) && go(rest, &n[1..]),
        }
    }
    go(&p, &n)
}

/// Folders Windows owns (removing a product never deletes them).
fn system_folders(plan: &Plan) -> Vec<String> {
    const SYSTEM: [&str; 20] = [
        "ROOTDRIVE",
        "WindowsFolder",
        "SystemFolder",
        "System64Folder",
        "ProgramFilesFolder",
        "ProgramFiles64Folder",
        "CommonFilesFolder",
        "CommonFiles64Folder",
        "CommonAppDataFolder",
        "AppDataFolder",
        "LocalAppDataFolder",
        "StartMenuFolder",
        "ProgramMenuFolder",
        "StartupFolder",
        "DesktopFolder",
        "PersonalFolder",
        "TempFolder",
        "FontsFolder",
        "TARGETDIR",
        "AdminToolsFolder",
    ];
    let mut out: Vec<String> = SYSTEM
        .iter()
        .filter_map(|p| plan.property(p).or_else(|| plan.directory(p)))
        .map(String::from)
        .collect();
    out.push(String::from("C:\\Windows\\Installer\\"));
    out.push(String::from("C:\\Users\\"));
    out.push(String::from("C:\\ProgramData\\Microsoft\\Windows\\"));
    out
}

/// msiexec's command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandLine {
    pub install: Option<String>,
    pub remove: Option<String>,
    pub repair: Option<(String, String)>,
    pub ui: u32,
    /// /passive and /qb-!: no Cancel button, no modal end.
    pub passive: bool,
    pub log: Option<String>,
    pub properties: Vec<(String, String)>,
    pub help: bool,
    /// /y (true: DllRegisterServer) or /z (DllUnregisterServer) of a
    /// module.
    pub self_reg: Option<(bool, String)>,
}
/// Parse msiexec's arguments (already split); Err is 1639.
pub fn parse_command_line(args: &[String]) -> Result<CommandLine, u32> {
    let mut a = CommandLine {
        ui: level::FULL,
        ..CommandLine::default()
    };
    let mut i = 0;
    // The splitter already took the quotes off.
    let unq = |s: &str| String::from(s);
    while i < args.len() {
        let raw = unq(&args[i]);
        let lower = raw.to_ascii_lowercase();
        let switch = lower.starts_with('/') || lower.starts_with('-');
        let body = if switch { &lower[1..] } else { lower.as_str() };
        if switch {
            match body {
                "i" | "package" => {
                    i += 1;
                    a.install = Some(unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?));
                }
                "x" | "uninstall" => {
                    i += 1;
                    a.remove = Some(unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?));
                }
                "quiet" | "qn" | "q" | "qn+" | "qn-" => {
                    a.ui = level::NONE;
                    a.passive = body == "quiet";
                }
                "passive" => {
                    a.ui = level::BASIC;
                    a.passive = true;
                }
                "qb" | "qb+" | "qb-" => a.ui = level::BASIC,
                "qb!" | "qb!-" | "qb-!" => {
                    a.ui = level::BASIC;
                    a.passive = true;
                }
                "qr" | "qr+" => a.ui = level::REDUCED,
                "qf" => a.ui = level::FULL,
                "norestart" => a
                    .properties
                    .push((String::from("REBOOT"), String::from("ReallySuppress"))),
                "forcerestart" => a
                    .properties
                    .push((String::from("REBOOT"), String::from("Force"))),
                "promptrestart" => (),
                "?" | "h" | "help" => a.help = true,
                "y" | "z" => {
                    i += 1;
                    a.self_reg = Some((
                        body == "y",
                        unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?),
                    ));
                }
                "log" => {
                    i += 1;
                    a.log = Some(unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?));
                }
                b if b.starts_with('l') => {
                    i += 1;
                    a.log = Some(unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?));
                }
                b if b.starts_with('f') => {
                    let mode = if b.len() > 1 {
                        String::from(&b[1..])
                    } else {
                        String::from("pecms")
                    };
                    if !mode.chars().all(|c| "pecdaumsvo".contains(c)) {
                        return Err(code::INVALID_COMMAND_LINE);
                    }
                    i += 1;
                    let target = unq(args.get(i).ok_or(code::INVALID_COMMAND_LINE)?);
                    a.repair = Some((mode, target));
                }
                // /i{GUID}, /x{GUID} without a space.
                b if b.starts_with("x{") => a.remove = Some(String::from(&raw[2..])),
                b if b.starts_with("i{") || (b.starts_with('i') && raw.len() > 2) => {
                    a.install = Some(String::from(&raw[2..]))
                }
                _ => return Err(code::INVALID_COMMAND_LINE),
            }
        } else {
            let (k, v) = raw.split_once('=').ok_or(code::INVALID_COMMAND_LINE)?;
            if k.is_empty()
                || !k
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            {
                return Err(code::INVALID_COMMAND_LINE);
            }
            a.properties.push((String::from(k), unq(v)));
        }
        i += 1;
    }
    if a.help {
        return Ok(a);
    }
    let chosen = [a.install.is_some(), a.remove.is_some(), a.repair.is_some()]
        .iter()
        .filter(|x| **x)
        .count();
    if chosen != 1 {
        return Err(code::INVALID_COMMAND_LINE);
    }
    Ok(a)
}

/// Split a Windows command line as CommandLineToArgvW does (quotes group,
/// `\"` is a quote).
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
            }
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' | '\t' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                    any = false;
                }
            }
            _ => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|s| s.to_owned()).collect()
}

/// msiexec.exe's own command-line rules (not CommandLineToArgvW's): a
/// quote opens or closes a quoted run, `""` inside one is a quote, and a
/// backslash is only a backslash, so `INSTALLDIR="C:\Program Files\App\"`
/// ends where it looks like it ends.
pub fn split_msiexec_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
            }
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' | '\t' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                    any = false;
                }
            }
            _ => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Windows Installer's help text (`msiexec /?`).
pub const HELP: &str = "Doors \u{ae} Installer. V 5.0.26300.1\r\n\r\nmsiexec /Option <Required Parameter> [Optional Parameter]\r\n\r\nInstall Options\r\n\t</package | /i> <Product.msi>\r\n\t\tInstalls or configures a product\r\n\t/a <Product.msi>\r\n\t\tAdministrative install - Installs a product on the network\r\n\t/j<u|m> <Product.msi> [/t <Transform List>] [/g <Language ID>]\r\n\t\tAdvertises a product - m to all users, u to current user\r\n\t</uninstall | /x> <Product.msi | ProductCode>\r\n\t\tUninstalls the product\r\nDisplay Options\r\n\t/quiet\r\n\t\tQuiet mode, no user interaction\r\n\t/passive\r\n\t\tUnattended mode - progress bar only\r\n\t/q[n|b|r|f]\r\n\t\tSets user interface level\r\n\t\tn - No UI\r\n\t\tb - Basic UI\r\n\t\tr - Reduced UI\r\n\t\tf - Full UI (default)\r\n\t/help\r\n\t\tHelp information\r\nRestart Options\r\n\t/norestart\r\n\t\tDo not restart after the installation is complete\r\n\t/promptrestart\r\n\t\tPrompts the user for restart if necessary\r\n\t/forcerestart\r\n\t\tAlways restart the computer after installation\r\nLogging Options\r\n\t/l[i|w|e|a|r|u|c|m|o|p|v|x|+|!|*] <LogFile>\r\n\t\ti - Status messages\r\n\t\tw - Nonfatal warnings\r\n\t\te - All error messages\r\n\t\ta - Start up of actions\r\n\t\tr - Action-specific records\r\n\t\tu - User requests\r\n\t\tc - Initial UI parameters\r\n\t\tm - Out-of-memory or fatal exit information\r\n\t\to - Out-of-disk-space messages\r\n\t\tp - Terminal properties\r\n\t\tv - Verbose output\r\n\t\tx - Extra debugging information\r\n\t\t+ - Append to existing log file\r\n\t\t! - Flush each line to the log\r\n\t\t* - Log all information, except for v and x options\r\n\t/log <LogFile>\r\n\t\tEquivalent of /l* <LogFile>\r\nRepair Options\r\n\t/f[p|e|c|m|s|o|d|a|u|v] <Product.msi | ProductCode>\r\n\t\tRepairs a product\r\n\t\tp - only if file is missing\r\n\t\to - if file is missing or an older version is installed (default)\r\n\t\te - if file is missing or an equal or older version is installed\r\n\t\td - if file is missing or a different version is installed\r\n\t\tc - if file is missing or checksum does not match the calculated value\r\n\t\ta - forces all files to be reinstalled\r\n\t\tu - all required user-specific registry entries (default)\r\n\t\tm - all required computer-specific registry entries (default)\r\n\t\ts - all existing shortcuts (default)\r\n\t\tv - runs from source and recaches local package\r\nSetting Public Properties\r\n\t[PROPERTY=PropertyValue]\r\n\r\nConsult the Doors \u{ae} Installer SDK for additional documentation on the\r\ncommand line syntax.\r\n\r\nCopyright \u{a9} Macrohard Corporation. All rights reserved.\r\n";

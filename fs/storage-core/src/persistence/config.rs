use super::{Access, Error, PersistentVolume, RegName, Result, Subject};
use crate::storage::Vfs;
/// Registry capacity per volume. The catalog keeps the first
/// `CATALOG_KEYS` keys and `CATALOG_VALUES` values at its fixed offsets (every
/// catalog format); later records live in the registry hive file
/// (`super::HIVE_NAME`, `codec::encode_hive`), written in the same journal
/// transaction as the catalog. The hive stays well inside one transaction:
/// at most 104 keys and 360 values, 82 KiB.
pub(crate) const KEYS: usize = 128;
pub(crate) const VALUES: usize = 384;
pub(crate) const CATALOG_KEYS: usize = 24;
pub(crate) const CATALOG_VALUES: usize = 24;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryRoot {
    LocalMachine,
    CurrentUser,
    ClassesRoot,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryView {
    Bits32,
    Bits64,
}
impl RegistryView {
    fn code(self) -> u8 {
        match self {
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RegistryType {
    None = 0,
    String = 1,
    ExpandString = 2,
    Binary = 3,
    Dword = 4,
    MultiString = 7,
    Qword = 11,
}
impl RegistryType {
    pub(crate) fn decode(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::String),
            2 => Ok(Self::ExpandString),
            3 => Ok(Self::Binary),
            4 => Ok(Self::Dword),
            7 => Ok(Self::MultiString),
            11 => Ok(Self::Qword),
            _ => Err(Error::Unsupported),
        }
    }
}
/// Value bytes one registry record holds. A longer value continues in
/// further records of the same key and name (`Entry::part` 1, 2, ...).
pub const RECORD_DATA: usize = 80;
/// The longest registry value Doors stores (26 records).
pub const MAX_VALUE_BYTES: usize = 2048;
/// One record's share of a value: the whole value when it fits a record,
/// otherwise one chunk (`total` says how long the whole value is).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Value {
    pub kind: RegistryType,
    pub(crate) len: u8,
    pub(crate) bytes: [u8; RECORD_DATA],
    /// The whole value's length (head record; `len` for an unchained one).
    pub(crate) total: u16,
}
impl Value {
    pub const EMPTY: Self = Self {
        kind: RegistryType::None,
        len: 0,
        bytes: [0; RECORD_DATA],
        total: 0,
    };
    /// A value that fits one record.
    pub fn new(kind: RegistryType, data: &[u8]) -> Result<Self> {
        if data.len() > RECORD_DATA {
            return Err(Error::BufferTooSmall);
        }
        Self::check(kind, data)?;
        Ok(Self::chunk(kind, data, data.len()))
    }
    /// One record of a value `total` bytes long (no type check: a chunk of
    /// a string need not end its string).
    pub(crate) fn chunk(kind: RegistryType, data: &[u8], total: usize) -> Self {
        let mut v = Self::EMPTY;
        v.kind = kind;
        v.len = data.len() as u8;
        v.bytes[..data.len()].copy_from_slice(data);
        v.total = total as u16;
        v
    }
    /// The whole value is well formed for its type.
    pub fn check(kind: RegistryType, data: &[u8]) -> Result<()> {
        if data.len() > MAX_VALUE_BYTES {
            return Err(Error::BufferTooSmall);
        }
        let good = match kind {
            RegistryType::Dword => data.len() == 4,
            RegistryType::Qword => data.len() == 8,
            RegistryType::String | RegistryType::ExpandString => {
                data.len() >= 2 && data.len().is_multiple_of(2) && data.ends_with(&[0, 0])
            }
            RegistryType::MultiString => {
                data.len() >= 4 && data.len().is_multiple_of(2) && data.ends_with(&[0, 0, 0, 0])
            }
            _ => true,
        };
        if !good {
            return Err(Error::TypeMismatch);
        }
        Ok(())
    }
    /// This record's bytes: the whole value unless `total` is longer.
    pub fn data(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
    /// The whole value's length.
    pub fn total(&self) -> usize {
        usize::from(self.total).max(usize::from(self.len))
    }
    pub fn dword(value: u32) -> Self {
        Self::new(RegistryType::Dword, &value.to_le_bytes()).unwrap()
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Key {
    pub id: u64,
    pub parent: u64,
    pub owner: u32,
    pub root: u8,
    pub view: u8,
    pub deleted: bool,
    pub name: RegName,
    pub version: u64,
}
impl Key {
    pub const EMPTY: Self = Self {
        id: 0,
        parent: 0,
        owner: 0,
        root: 0,
        view: 0,
        deleted: false,
        name: RegName::EMPTY,
        version: 0,
    };
}
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub key: u64,
    pub name: RegName,
    pub value: Value,
    /// 0 for a value's first record, 1.. for its continuations.
    pub part: u8,
}
impl Entry {
    pub const EMPTY: Self = Self {
        key: 0,
        name: RegName::EMPTY,
        value: Value::EMPTY,
        part: 0,
    };
    /// The first record of value `name` of `key`.
    pub fn head_of(&self, key: u64, name: RegName) -> bool {
        self.key == key && self.part == 0 && self.name.same(name, true)
    }
}
/// Display scale preference: 0 = automatic (the default), otherwise a
/// Windows display scale of 100..=300 percent in steps of 25.
pub(crate) const fn valid_scale(value: u32) -> bool {
    value == 0 || (value >= 100 && value <= 300 && value.is_multiple_of(25))
}
#[derive(Clone, Copy)]
pub(crate) struct Settings {
    pub owner: u32,
    pub schema: u32,
    pub theme: u32,
    pub scale: u32,
    pub wallpaper: u32,
    pub version: u64,
}
impl Settings {
    pub const EMPTY: Self = Self {
        owner: 0,
        schema: 0,
        theme: 0,
        scale: 0,
        wallpaper: 0,
        version: 0,
    };
}
#[derive(Clone, Copy)]
struct OpenKey {
    token: u64,
    key: u64,
    fallback: u64,
    access: Access,
}
impl OpenKey {
    const EMPTY: Self = Self {
        token: 0,
        key: 0,
        fallback: 0,
        access: Access(0),
    };
}
pub(crate) struct Store {
    pub keys: [Key; KEYS],
    pub values: [Entry; VALUES],
    pub settings: [Settings; 8],
    pub next_key: u64,
    opens: [OpenKey; 16],
    next_handle: u64,
}
impl Store {
    pub const fn new() -> Self {
        Self {
            keys: [Key::EMPTY; KEYS],
            values: [Entry::EMPTY; VALUES],
            settings: [Settings::EMPTY; 8],
            next_key: 1,
            opens: [OpenKey::EMPTY; 16],
            next_handle: 1,
        }
    }
    pub fn clear_handles(&mut self) {
        self.opens.fill(OpenKey::EMPTY);
        self.next_handle = 1;
        for key in &mut self.keys {
            if key.deleted {
                *key = Key::EMPTY;
            }
        }
    }
    /// Deleted ancestors still carry the scope/identity chain for outstanding
    /// deleted descendants. Retire leaves only after their last handle closes,
    /// then reconsider ancestors. Each pass removes a record, so work is bounded
    /// by the catalog capacity and never recursively consumes a kernel stack.
    fn collect_deleted(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..KEYS {
            let candidate = self.keys.iter().position(|key| {
                key.id != 0
                    && key.deleted
                    && !self.opens.iter().any(|open| {
                        open.token != 0 && (open.key == key.id || open.fallback == key.id)
                    })
                    && !self
                        .keys
                        .iter()
                        .any(|child| child.id != 0 && child.parent == key.id)
            });
            let Some(index) = candidate else { break };
            self.keys[index] = Key::EMPTY;
            changed = true;
        }
        changed
    }
}
pub(crate) fn initialize(_store: &mut Store) -> Result<()> {
    Ok(())
}
#[derive(Debug)]
pub struct KeyHandle {
    mount: u64,
    token: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Setting {
    Theme,
    ScalePercent,
    DesktopAppearance,
    DesktopControls,
    /// `desktop::ShellLayout` flags (pins masks, Start All view, sort).
    ShellLayout,
    /// `desktop::ShellLayout` order (pin order permutation ranks).
    ShellOrder,
    /// Installed input methods and the current one
    /// (`door_keyboard_layouts::Preferences` bits).
    InputMethods,
    /// Time zone (`door_locale::tz::Zone::id`; default UTC). Per user, like
    /// the other native settings (Windows keeps it machine-wide).
    TimeZone,
    /// Regional format (`door_locale::region::Region::id`; default en-US).
    RegionalFormat,
    /// System accent colour (`door_service_protocol::desktop::Accent` word;
    /// default 0 = Door's default blue).
    AccentColor,
    /// A P14 preference word (`door_service_protocol::preferences`, ids
    /// 10 and up): an HKCU dword with the table's name, default and
    /// validator. Construct with [`Setting::from_id`].
    Preference(u32),
}
/// A per-user registry dword setting: (value name, default, validator).
type RegistryDword = (&'static str, u32, fn(u32) -> bool);
/// The HKCU dword of `Setting::DesktopAppearance` (bit 0 dark, bits 1.. the
/// picture), read through to the settings record until first written.
const APPEARANCE_DWORD: &str = "DesktopAppearance";
impl Setting {
    /// The setting with wire id `id` (0..=9 the native settings, 10 and up
    /// the preference table).
    pub fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0 => Self::Theme,
            1 => Self::ScalePercent,
            2 => Self::DesktopAppearance,
            3 => Self::DesktopControls,
            4 => Self::ShellLayout,
            5 => Self::ShellOrder,
            6 => Self::InputMethods,
            7 => Self::TimeZone,
            8 => Self::RegionalFormat,
            9 => Self::AccentColor,
            _ => {
                door_service_protocol::preferences::find(id)?;
                Self::Preference(id)
            }
        })
    }
    /// Per-user registry dword settings: (value name, default, validator).
    fn registry(self) -> Option<RegistryDword> {
        match self {
            Self::Preference(id) => {
                door_service_protocol::preferences::find(id).map(|p| (p.name, p.default, p.valid))
            }
            Self::DesktopControls => Some((
                "DesktopControls",
                door_service_protocol::desktop::Controls::DEFAULT.bits(),
                |v| door_service_protocol::desktop::Controls::from_bits(v).is_some(),
            )),
            Self::ShellLayout => Some((
                "ShellLayout",
                0,
                door_service_protocol::desktop::ShellLayout::flags_valid,
            )),
            Self::ShellOrder => Some((
                "ShellOrder",
                0,
                door_service_protocol::desktop::ShellLayout::order_valid,
            )),
            Self::InputMethods => Some((
                "InputMethods",
                door_keyboard_layouts::Preferences::DEFAULT.bits(),
                |v| door_keyboard_layouts::Preferences::from_bits(v).is_some(),
            )),
            Self::TimeZone => Some((
                "TimeZone",
                door_locale::tz::DEFAULT_ID as u32,
                door_locale::tz::valid,
            )),
            Self::RegionalFormat => Some((
                "RegionalFormat",
                door_locale::region::DEFAULT_ID as u32,
                door_locale::region::valid,
            )),
            Self::AccentColor => Some((
                "AccentColor",
                door_service_protocol::desktop::Accent::DEFAULT.bits(),
                |v| door_service_protocol::desktop::Accent::from_bits(v).is_some(),
            )),
            _ => None,
        }
    }
}
/// A value name (empty: the key's default value).
fn name(units: &[u16]) -> Result<RegName> {
    if units.is_empty() {
        Ok(RegName::EMPTY)
    } else {
        RegName::value(units)
    }
}
fn parse(units: &[u16]) -> Result<([RegName; 16], usize)> {
    if units.len() > 768 || units.contains(&0) {
        return Err(Error::InvalidName);
    }
    let mut names = [RegName::EMPTY; 16];
    let mut n = 0;
    for part in units.split(|&u| u == 92) {
        if part.is_empty() {
            continue;
        }
        if n == 16 {
            return Err(Error::InvalidName);
        }
        names[n] = RegName::key(part)?;
        n += 1;
    }
    Ok((names, n))
}
impl<F: Vfs> PersistentVolume<'_, F> {
    /// A value under key `index` changed: notify that user's settings
    /// subscribers (HKCU, root 2), or machine subscribers (user 0).
    fn note_key(&mut self, index: usize) {
        let key = self.state.config.keys[index];
        let user = if key.root == 2 { key.owner } else { 0 };
        self.note(super::namespace::Change::Settings(user));
    }
    fn key_index(&self, id: u64) -> Result<usize> {
        self.state
            .config
            .keys
            .iter()
            .position(|k| k.id == id && id != 0)
            .ok_or(Error::NotFound)
    }
    fn key_handle(&self, handle: &KeyHandle, access: Access) -> Result<OpenKey> {
        self.ready()?;
        if handle.mount != self.state.mount_identity() {
            return Err(Error::InvalidHandle);
        }
        let o = self
            .state
            .config
            .opens
            .iter()
            .find(|o| o.token == handle.token && o.token != 0)
            .copied()
            .ok_or(Error::InvalidHandle)?;
        if !o.access.contains(access) {
            return Err(Error::AccessDenied);
        }
        for id in [o.key, o.fallback] {
            if id != 0 && self.state.config.keys[self.key_index(id)?].deleted {
                return Err(Error::KeyDeleted);
            }
        }
        Ok(o)
    }
    fn find_key(&self, parent: u64, part: RegName) -> Option<usize> {
        self.state
            .config
            .keys
            .iter()
            .position(|k| k.id != 0 && !k.deleted && k.parent == parent && k.name.same(part, true))
    }
    fn root_key(&self, root: u8, owner: u32, view: u8) -> Option<usize> {
        self.state.config.keys.iter().position(|k| {
            k.id != 0
                && !k.deleted
                && k.parent == 0
                && k.root == root
                && k.owner == owner
                && k.view == view
        })
    }
    fn registry_path(
        &mut self,
        subject: Subject,
        root: u8,
        view: u8,
        names: &[RegName],
        create: bool,
    ) -> Result<u64> {
        let owner = if root == 1 { 0 } else { subject.id };
        let existing = self.root_key(root, owner, view);
        let mut cursor = existing;
        let mut insertion_parent = None;
        let mut missing = usize::from(existing.is_none());
        for &part in names {
            let next = cursor.and_then(|i| self.find_key(self.state.config.keys[i].id, part));
            if next.is_none() && cursor.is_some() {
                insertion_parent = cursor;
            }
            cursor = next;
            if cursor.is_none() {
                missing += 1;
            }
        }
        if missing == 0 {
            return Ok(self.state.config.keys[cursor.unwrap()].id);
        }
        if !create {
            return Err(Error::NotFound);
        }
        if root == 1 && !subject.administrator() {
            return Err(Error::AccessDenied);
        }
        if self.state.config.keys.iter().filter(|k| k.id == 0).count() < missing {
            return Err(Error::NoSpace);
        }
        self.state
            .config
            .next_key
            .checked_add(missing as u64)
            .ok_or(Error::NoSpace)?;
        if let Some(index) = insertion_parent {
            self.state.config.keys[index]
                .version
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
        }
        self.mutation()?;
        let mut parent = if let Some(i) = existing {
            self.state.config.keys[i].id
        } else {
            self.insert_key(0, owner, root, view, RegName::EMPTY)?
        };
        for &part in names {
            parent = if let Some(i) = self.find_key(parent, part) {
                self.state.config.keys[i].id
            } else {
                self.insert_key(parent, owner, root, view, part)?
            };
        }
        Ok(parent)
    }
    fn insert_key(
        &mut self,
        parent: u64,
        owner: u32,
        root: u8,
        view: u8,
        name: RegName,
    ) -> Result<u64> {
        let index = self
            .state
            .config
            .keys
            .iter()
            .position(|k| k.id == 0)
            .ok_or(Error::NoSpace)?;
        let id = self.state.config.next_key;
        self.state.config.next_key = id.checked_add(1).ok_or(Error::NoSpace)?;
        self.state.config.keys[index] = Key {
            id,
            parent,
            owner,
            root,
            view,
            deleted: false,
            name,
            version: 1,
        };
        if parent != 0 {
            let p = self.key_index(parent)?;
            self.state.config.keys[p].version = self.state.config.keys[p]
                .version
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
        }
        Ok(id)
    }
    pub fn registry_open(
        &mut self,
        subject: Subject,
        root: RegistryRoot,
        view: RegistryView,
        path: &[u16],
        access: Access,
        create: bool,
    ) -> Result<KeyHandle> {
        self.ready()?;
        if !access.valid() || access.0 & !(1 | 2 | 4) != 0 {
            return Err(Error::InvalidParameter);
        }
        let slot = self
            .state
            .config
            .opens
            .iter()
            .position(|o| o.token == 0)
            .ok_or(Error::NoSpace)?;
        let token = self.state.config.next_handle;
        let next = token.checked_add(1).ok_or(Error::NoSpace)?;
        // HKLM is written by administrators (an elevated process) and the
        // system, as Windows' default HKLM\SOFTWARE security allows.
        if root == RegistryRoot::LocalMachine && !subject.administrator() && access.0 & (2 | 4) != 0
        {
            return Err(Error::AccessDenied);
        }
        let (names, count) = parse(path)?;
        let writing = create || access.0 & (2 | 4) != 0;
        let (key, fallback) = if root == RegistryRoot::ClassesRoot && writing {
            // Writing HKCR: Windows writes HKLM\Software\Classes for an
            // administrator and HKCU\Software\Classes for anyone else.
            if count > 14 {
                return Err(Error::InvalidName);
            }
            let mut full = [RegName::EMPTY; 16];
            full[0] = RegName::ascii("Software")?;
            full[1] = RegName::ascii("Classes")?;
            full[2..2 + count].copy_from_slice(&names[..count]);
            let hive = if subject.administrator() { 1 } else { 2 };
            (
                self.registry_path(subject, hive, view.code(), &full[..count + 2], create)?,
                0,
            )
        } else if root == RegistryRoot::ClassesRoot {
            if count > 14 {
                return Err(Error::InvalidName);
            }
            let mut full = [RegName::EMPTY; 16];
            full[0] = RegName::ascii("Software")?;
            full[1] = RegName::ascii("Classes")?;
            full[2..2 + count].copy_from_slice(&names[..count]);
            // An elevated program's COM lookups (CLSID, AppID, TypeLib,
            // Interface) see the machine's registrations only, as Windows'
            // COM ignores per-user classes for an elevated caller: a class
            // an unelevated program registered for its user must not start
            // under Administrators.
            let machine_only = subject.administrator()
                && !subject.system()
                && count > 0
                && ["CLSID", "AppID", "TypeLib", "Interface"]
                    .iter()
                    .any(|k| RegName::ascii(k).is_ok_and(|k| names[0].same(k, true)));
            let user = if machine_only {
                None
            } else {
                self.registry_path(subject, 2, view.code(), &full[..count + 2], false)
                    .ok()
            };
            let machine = self
                .registry_path(subject, 1, view.code(), &full[..count + 2], false)
                .ok();
            match (user, machine) {
                (Some(a), b) => (a, b.unwrap_or(0)),
                (None, Some(b)) => (b, 0),
                _ => return Err(Error::NotFound),
            }
        } else {
            (
                self.registry_path(
                    subject,
                    if root == RegistryRoot::LocalMachine {
                        1
                    } else {
                        2
                    },
                    view.code(),
                    &names[..count],
                    create,
                )?,
                0,
            )
        };
        self.state.config.next_handle = next;
        self.state.config.opens[slot] = OpenKey {
            token,
            key,
            fallback,
            access,
        };
        Ok(KeyHandle {
            mount: self.state.mount_identity(),
            token,
        })
    }
    /// The key holding value `name` for this open key (its own, else the
    /// merged HKCR fallback).
    fn value_key(&self, o: &OpenKey, name: RegName) -> Option<u64> {
        [o.key, o.fallback].into_iter().find(|&key| {
            key != 0
                && self
                    .state
                    .config
                    .values
                    .iter()
                    .any(|e| e.head_of(key, name))
        })
    }
    /// The value's first record: the whole value when it fits one record
    /// (`Value::total` says how long it is; `registry_read` reads longer
    /// ones).
    pub fn registry_get(&self, handle: &KeyHandle, value_name: &[u16]) -> Result<Value> {
        let o = self.key_handle(handle, Access::READ)?;
        let name = name(value_name)?;
        let key = self.value_key(&o, name).ok_or(Error::NotFound)?;
        self.state
            .config
            .values
            .iter()
            .find(|e| e.head_of(key, name))
            .map(|e| e.value)
            .ok_or(Error::NotFound)
    }
    /// Copies the value's bytes from `offset` into `out`: (type, whole
    /// length, bytes copied).
    pub fn registry_read(
        &self,
        handle: &KeyHandle,
        value_name: &[u16],
        offset: usize,
        out: &mut [u8],
    ) -> Result<(RegistryType, usize, usize)> {
        let o = self.key_handle(handle, Access::READ)?;
        let name = name(value_name)?;
        let key = self.value_key(&o, name).ok_or(Error::NotFound)?;
        let head = self.registry_get(handle, value_name)?;
        let total = head.total();
        let mut copied = 0;
        for part in 0..=total.saturating_sub(1) / RECORD_DATA {
            let start = part * RECORD_DATA;
            if start + RECORD_DATA <= offset {
                continue;
            }
            if start >= offset + out.len() || copied == out.len() {
                break;
            }
            let e = self
                .state
                .config
                .values
                .iter()
                .find(|e| e.key == key && e.part as usize == part && e.name.same(name, true))
                .ok_or(Error::Corrupt)?;
            let data = e.value.data();
            let from = offset.saturating_sub(start).min(data.len());
            let n = (data.len() - from).min(out.len() - copied);
            out[copied..copied + n].copy_from_slice(&data[from..from + n]);
            copied += n;
        }
        Ok((head.kind, total, copied))
    }
    pub fn registry_set(
        &mut self,
        handle: &KeyHandle,
        value_name: &[u16],
        value: Value,
    ) -> Result<()> {
        self.registry_set_data(handle, value_name, value.kind, value.data())
    }
    /// Sets a value of up to `MAX_VALUE_BYTES`: one record per
    /// `RECORD_DATA` bytes, replacing every record of the old value.
    pub fn registry_set_data(
        &mut self,
        handle: &KeyHandle,
        value_name: &[u16],
        kind: RegistryType,
        data: &[u8],
    ) -> Result<()> {
        let o = self.key_handle(handle, Access::WRITE)?;
        let name = name(value_name)?;
        Value::check(kind, data)?;
        let parts = data.len().div_ceil(RECORD_DATA).max(1);
        let usable = self
            .state
            .config
            .values
            .iter()
            .filter(|e| e.key == 0 || (e.key == o.key && e.name.same(name, true)))
            .count();
        if usable < parts {
            return Err(Error::NoSpace);
        }
        let index = self.key_index(o.key)?;
        let version = self.state.config.keys[index]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        // The old value's head slot keeps its place in enumeration order.
        let head = self
            .state
            .config
            .values
            .iter()
            .position(|e| e.head_of(o.key, name));
        for e in &mut self.state.config.values {
            if e.key == o.key && e.name.same(name, true) {
                *e = Entry::EMPTY;
            }
        }
        let mut free = head
            .into_iter()
            .chain((0..self.state.config.values.len()).filter(move |&i| Some(i) != head));
        for part in 0..parts {
            let slot = loop {
                let slot = free.next().ok_or(Error::NoSpace)?;
                if self.state.config.values[slot].key == 0 {
                    break slot;
                }
            };
            let chunk = &data
                [(part * RECORD_DATA).min(data.len())..((part + 1) * RECORD_DATA).min(data.len())];
            self.state.config.values[slot] = Entry {
                key: o.key,
                name,
                value: Value::chunk(kind, chunk, if part == 0 { data.len() } else { 0 }),
                part: part as u8,
            };
        }
        self.state.config.keys[index].version = version;
        self.note_key(index);
        Ok(())
    }
    pub fn registry_delete_value(&mut self, handle: &KeyHandle, value_name: &[u16]) -> Result<()> {
        let o = self.key_handle(handle, Access::WRITE)?;
        let name = name(value_name)?;
        if !self
            .state
            .config
            .values
            .iter()
            .any(|e| e.head_of(o.key, name))
        {
            return Err(Error::NotFound);
        }
        let index = self.key_index(o.key)?;
        let version = self.state.config.keys[index]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        for e in &mut self.state.config.values {
            if e.key == o.key && e.name.same(name, true) {
                *e = Entry::EMPTY;
            }
        }
        self.state.config.keys[index].version = version;
        self.note_key(index);
        Ok(())
    }
    pub fn registry_delete_key(&mut self, handle: &KeyHandle) -> Result<()> {
        let o = self.key_handle(handle, Access::DELETE)?;
        let index = self.key_index(o.key)?;
        if self.state.config.keys[index].parent == 0
            || self
                .state
                .config
                .keys
                .iter()
                .any(|k| k.id != 0 && !k.deleted && k.parent == o.key)
        {
            return Err(Error::InvalidParameter);
        }
        let parent = self.key_index(self.state.config.keys[index].parent)?;
        let version = self.state.config.keys[parent]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.config.keys[parent].version = version;
        self.state.config.keys[index].deleted = true;
        for e in &mut self.state.config.values {
            if e.key == o.key {
                *e = Entry::EMPTY;
            }
        }
        self.note_key(index);
        Ok(())
    }
    pub fn registry_close(&mut self, handle: KeyHandle) -> Result<()> {
        self.ready()?;
        if handle.mount != self.state.mount_identity() {
            return Err(Error::InvalidHandle);
        }
        let slot = self
            .state
            .config
            .opens
            .iter()
            .position(|o| o.token == handle.token && o.token != 0)
            .ok_or(Error::InvalidHandle)?;
        self.state.config.opens[slot] = OpenKey::EMPTY;
        self.dirty |= self.state.config.collect_deleted();
        Ok(())
    }
    /// Versioned polling registration: capture then compare under the same owner.
    pub fn registry_version(&self, handle: &KeyHandle) -> Result<u64> {
        let o = self.key_handle(handle, Access::READ)?;
        let a = self.state.config.keys[self.key_index(o.key)?].version;
        let b = if o.fallback == 0 {
            0
        } else {
            self.state.config.keys[self.key_index(o.fallback)?].version
        };
        a.checked_add(b).ok_or(Error::NoSpace)
    }
    pub fn registry_changed(&self, handle: &KeyHandle, observed: u64) -> Result<bool> {
        Ok(self.registry_version(handle)? != observed)
    }
    pub fn registry_enumerate_value(
        &self,
        handle: &KeyHandle,
        index: usize,
        observed: u64,
    ) -> Result<(RegName, Value)> {
        if self.registry_version(handle)? != observed {
            return Err(Error::Changed);
        }
        let o = self.key_handle(handle, Access::READ)?;
        let mut position = 0;
        for key in [o.key, o.fallback] {
            if key == 0 {
                continue;
            }
            for e in &self.state.config.values {
                if e.key != key || e.part != 0 {
                    continue;
                }
                if key == o.fallback
                    && self
                        .state
                        .config
                        .values
                        .iter()
                        .any(|p| p.head_of(o.key, e.name))
                {
                    continue;
                }
                if position == index {
                    return Ok((e.name, e.value));
                }
                position += 1;
            }
        }
        Err(Error::NotFound)
    }
    pub fn registry_enumerate_key(
        &self,
        handle: &KeyHandle,
        index: usize,
        observed: u64,
    ) -> Result<RegName> {
        if self.registry_version(handle)? != observed {
            return Err(Error::Changed);
        }
        let open = self.key_handle(handle, Access::READ)?;
        let mut position = 0;
        for parent in [open.key, open.fallback] {
            if parent == 0 {
                continue;
            }
            for key in &self.state.config.keys {
                if key.id == 0 || key.deleted || key.parent != parent {
                    continue;
                }
                if parent == open.fallback
                    && self.state.config.keys.iter().any(|k| {
                        k.id != 0
                            && !k.deleted
                            && k.parent == open.key
                            && k.name.same(key.name, true)
                    })
                {
                    continue;
                }
                if position == index {
                    return Ok(key.name);
                }
                position += 1;
            }
        }
        Err(Error::NotFound)
    }
    pub fn setting_get(&self, subject: Subject, owner: u32, setting: Setting) -> Result<u32> {
        self.ready()?;
        if owner != subject.id && !subject.system() {
            return Err(Error::AccessDenied);
        }
        if let Some((value_name, default, valid)) = setting.registry() {
            let Some(root) = self.root_key(2, owner, 64) else {
                return Ok(default);
            };
            let name = RegName::ascii(value_name)?;
            let key = self.state.config.keys[root].id;
            let Some(entry) = self
                .state
                .config
                .values
                .iter()
                .find(|v| v.head_of(key, name))
            else {
                return Ok(default);
            };
            if entry.value.kind != RegistryType::Dword || entry.value.data().len() != 4 {
                return Err(Error::TypeMismatch);
            }
            let value = u32::from_le_bytes(entry.value.data().try_into().unwrap());
            return if valid(value) {
                Ok(value)
            } else {
                Err(Error::InvalidParameter)
            };
        }
        // The desktop appearance is a per-user dword (the typed settings
        // registry persists every key as one); volumes from before keep it
        // in the settings record until the first write.
        if setting == Setting::DesktopAppearance
            && let Some(value) = self.registry_dword_get(owner, APPEARANCE_DWORD)?
        {
            return if value < door_service_protocol::window::APPEARANCE_VALUES {
                Ok(value)
            } else {
                Err(Error::InvalidParameter)
            };
        }
        let found = self
            .state
            .config
            .settings
            .iter()
            .find(|s| s.schema != 0 && s.owner == owner);
        Ok(match setting {
            Setting::Theme => found.map_or(0, |s| s.theme),
            // 0 = automatic (host hint, else the integer heuristic).
            Setting::ScalePercent => found.map_or(0, |s| s.scale),
            Setting::DesktopAppearance => {
                found.map_or(0, |s| (s.wallpaper << 1) | u32::from(s.theme != 0))
            }
            Setting::DesktopControls
            | Setting::ShellLayout
            | Setting::ShellOrder
            | Setting::InputMethods
            | Setting::TimeZone
            | Setting::RegionalFormat
            | Setting::AccentColor
            | Setting::Preference(_) => {
                unreachable!("generic registry setting handled above")
            }
        })
    }
    pub fn setting_set(
        &mut self,
        subject: Subject,
        owner: u32,
        setting: Setting,
        value: u32,
    ) -> Result<()> {
        self.ready()?;
        if owner != subject.id && !subject.system() {
            return Err(Error::AccessDenied);
        }
        if let Some((value_name, _, valid)) = setting.registry() {
            if !valid(value) {
                return Err(Error::InvalidParameter);
            }
            self.registry_dword_set(owner, value_name, value)?;
            self.note(super::namespace::Change::Settings(owner));
            return Ok(());
        }
        if !match setting {
            Setting::Theme => value <= 2,
            Setting::ScalePercent => valid_scale(value),
            Setting::DesktopAppearance => value < door_service_protocol::window::APPEARANCE_VALUES,
            Setting::DesktopControls
            | Setting::ShellLayout
            | Setting::ShellOrder
            | Setting::InputMethods
            | Setting::TimeZone
            | Setting::RegionalFormat
            | Setting::AccentColor
            | Setting::Preference(_) => {
                unreachable!("generic registry setting handled above")
            }
        } {
            return Err(Error::InvalidParameter);
        }
        // The appearance dword first (its capacity checks precede any
        // mutation); the record keeps the shared dark bit for Theme. A Theme
        // change moves the dword's dark bit once the dword exists.
        match setting {
            Setting::DesktopAppearance => {
                self.registry_dword_set(owner, APPEARANCE_DWORD, value)?
            }
            Setting::Theme => {
                if let Some(current) = self.registry_dword_get(owner, APPEARANCE_DWORD)?
                    && current < door_service_protocol::window::APPEARANCE_VALUES
                {
                    let dark = (current & !1) | u32::from(value != 0);
                    if dark != current {
                        self.registry_dword_set(owner, APPEARANCE_DWORD, dark)?;
                    }
                }
            }
            _ => (),
        }
        let slot = self
            .state
            .config
            .settings
            .iter()
            .position(|s| s.schema != 0 && s.owner == owner)
            .or_else(|| {
                self.state
                    .config
                    .settings
                    .iter()
                    .position(|s| s.schema == 0)
            })
            .ok_or(Error::NoSpace)?;
        let old = self.state.config.settings[slot];
        let version = old.version.checked_add(1).ok_or(Error::NoSpace)?;
        self.mutation()?;
        let mut next = if old.schema == 0 {
            Settings {
                owner,
                schema: 4,
                theme: 0,
                scale: 0,
                wallpaper: 0,
                version,
            }
        } else {
            Settings { version, ..old }
        };
        match setting {
            Setting::Theme => next.theme = value,
            Setting::ScalePercent => next.scale = value,
            Setting::DesktopAppearance => {
                next.theme = value & 1;
                next.wallpaper = value >> 1;
            }
            Setting::DesktopControls
            | Setting::ShellLayout
            | Setting::ShellOrder
            | Setting::InputMethods
            | Setting::TimeZone
            | Setting::RegionalFormat
            | Setting::AccentColor
            | Setting::Preference(_) => {
                unreachable!("generic registry setting handled above")
            }
        };
        self.state.config.settings[slot] = next;
        self.note(super::namespace::Change::Settings(owner));
        Ok(())
    }
    /// The raw per-user HKCU dword `value_name`, if one is stored.
    pub(super) fn registry_dword_get(&self, owner: u32, value_name: &str) -> Result<Option<u32>> {
        let Some(root) = self.root_key(2, owner, 64) else {
            return Ok(None);
        };
        let name = RegName::ascii(value_name)?;
        let key = self.state.config.keys[root].id;
        let Some(entry) = self
            .state
            .config
            .values
            .iter()
            .find(|v| v.head_of(key, name))
        else {
            return Ok(None);
        };
        if entry.value.kind != RegistryType::Dword || entry.value.data().len() != 4 {
            return Err(Error::TypeMismatch);
        }
        Ok(Some(u32::from_le_bytes(
            entry.value.data().try_into().unwrap(),
        )))
    }
    /// Store a validated per-user registry dword under HKCU (catalog v1-v3
    /// registry records; no settings schema bump).
    pub(super) fn registry_dword_set(
        &mut self,
        owner: u32,
        value_name: &str,
        value: u32,
    ) -> Result<()> {
        let name = RegName::ascii(value_name)?;
        let root = self.root_key(2, owner, 64);
        let key = root.map(|i| self.state.config.keys[i].id);
        let slot = self
            .state
            .config
            .values
            .iter()
            .position(|e| key.is_some_and(|k| e.head_of(k, name)))
            .or_else(|| self.state.config.values.iter().position(|e| e.key == 0))
            .ok_or(Error::NoSpace)?;
        let version = if let Some(root) = root {
            self.state.config.keys[root]
                .version
                .checked_add(1)
                .ok_or(Error::NoSpace)?
        } else {
            if !self.state.config.keys.iter().any(|k| k.id == 0) {
                return Err(Error::NoSpace);
            }
            self.state
                .config
                .next_key
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
            2
        };
        // All capacity/counter checks precede mutation; existing catalog v1-v3
        // registry records carry the preference without a settings schema bump.
        self.mutation()?;
        let key = if let Some(key) = key {
            key
        } else {
            self.insert_key(0, owner, 2, 64, RegName::EMPTY)?
        };
        self.state.config.values[slot] = Entry {
            key,
            name,
            value: Value::dword(value),
            part: 0,
        };
        let root = self.key_index(key)?;
        self.state.config.keys[root].version = version;
        Ok(())
    }
}

//! A storage service stand-in shared by several "processes" (clients).
//! It validates `Setting` words on read and write as storage-core does,
//! keeps the Theme/DesktopAppearance dark-bit coupling, denies HKLM writes
//! to non-system clients, counts a change sequence for watches and can be
//! restarted, poisoned or made to fail writes.
#![allow(dead_code)]
use door_service_client::{RegistryRoot, Setting};
use door_settings_registry::{Backend, BackendError, Schema, Store, WatchState};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Setting(u32),
    Registry(u8, String, String),
}
pub fn slot(store: Store) -> Slot {
    match store {
        Store::Setting(setting) => Slot::Setting(setting as u32),
        Store::Registry { root, path, name } => Slot::Registry(
            root as u8,
            path.to_ascii_lowercase(),
            name.to_ascii_lowercase(),
        ),
    }
}

pub struct Shared {
    schema: &'static Schema,
    pub values: HashMap<Slot, u32>,
    pub corrupt: HashSet<Slot>,
    pub unreadable: HashSet<Slot>,
    pub fail_writes: u32,
    pub sequence: u64,
    pub generation: u64,
    pub writes: Vec<(Slot, u32)>,
    next_token: u64,
}
impl Shared {
    pub fn new(schema: &'static Schema) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            schema,
            values: HashMap::new(),
            corrupt: HashSet::new(),
            unreadable: HashSet::new(),
            fail_writes: 0,
            sequence: 1,
            generation: 1,
            writes: Vec::new(),
            next_token: 1,
        }))
    }
    fn validator(&self, store: Store) -> Option<fn(u32) -> bool> {
        match store {
            Store::Setting(_) => self
                .schema
                .words
                .iter()
                .find(|w| w.store == store)
                .map(|w| w.valid),
            // The registry stores any dword.
            Store::Registry { .. } => None,
        }
    }
    /// Another writer (or a damaged disk) stores raw bits, bypassing
    /// validation; watchers see a change.
    pub fn poke(&mut self, store: Store, value: u32) {
        self.values.insert(slot(store), value);
        self.sequence += 1;
    }
    pub fn raw(&self, store: Store) -> Option<u32> {
        self.values.get(&slot(store)).copied()
    }
    /// The service restarts: every connection and watch token dies.
    pub fn restart(&mut self) {
        self.generation += 1;
    }
}

pub struct Client {
    shared: Rc<RefCell<Shared>>,
    pub system: bool,
    generation: u64,
    tokens: HashMap<u64, u64>,
}
impl Client {
    pub fn new(shared: &Rc<RefCell<Shared>>, system: bool) -> Self {
        let generation = shared.borrow().generation;
        Self {
            shared: shared.clone(),
            system,
            generation,
            tokens: HashMap::new(),
        }
    }
    /// Connect again after a restart (tokens die with the connection).
    pub fn reconnect(&mut self) {
        self.generation = self.shared.borrow().generation;
        self.tokens.clear();
    }
    fn live(&self) -> Result<(), BackendError> {
        if self.generation == self.shared.borrow().generation {
            Ok(())
        } else {
            Err(BackendError::Disconnected)
        }
    }
}
impl Backend for Client {
    fn read(&mut self, store: Store) -> Result<u32, BackendError> {
        self.live()?;
        let shared = self.shared.borrow();
        let key = slot(store);
        if shared.corrupt.contains(&key) {
            return Err(BackendError::Corrupt);
        }
        if shared.unreadable.contains(&key) {
            return Err(BackendError::Denied);
        }
        let Some(value) = shared.values.get(&key).copied() else {
            return match store {
                Store::Setting(_) => Ok(shared
                    .schema
                    .words
                    .iter()
                    .find(|w| w.store == store)
                    .map_or(0, |w| w.default)),
                Store::Registry { .. } => Err(BackendError::NotFound),
            };
        };
        match shared.validator(store) {
            Some(valid) if !valid(value) => Err(BackendError::Invalid),
            _ => Ok(value),
        }
    }
    fn write(&mut self, store: Store, value: u32) -> Result<(), BackendError> {
        self.live()?;
        let mut shared = self.shared.borrow_mut();
        let key = slot(store);
        if let Store::Registry {
            root: RegistryRoot::LocalMachine,
            ..
        } = store
            && !self.system
        {
            return Err(BackendError::Denied);
        }
        if shared.corrupt.contains(&key) {
            return Err(BackendError::Corrupt);
        }
        if shared.fail_writes > 0 {
            shared.fail_writes -= 1;
            return Err(BackendError::Io);
        }
        if let Some(valid) = shared.validator(store)
            && !valid(value)
        {
            return Err(BackendError::Invalid);
        }
        shared.values.insert(key.clone(), value);
        // storage-core keeps one dark bit for Theme and DesktopAppearance.
        let theme = slot(Store::Setting(Setting::Theme));
        let appearance = slot(Store::Setting(Setting::DesktopAppearance));
        if key == theme {
            let old = shared.values.get(&appearance).copied().unwrap_or(0);
            shared
                .values
                .insert(appearance, (old & !1) | u32::from(value != 0));
        } else if key == appearance {
            shared.values.insert(theme, value & 1);
        }
        shared.writes.push((key, value));
        shared.sequence += 1;
        Ok(())
    }
    fn watch(&mut self) -> Result<u64, BackendError> {
        self.live()?;
        let mut shared = self.shared.borrow_mut();
        let token = shared.next_token;
        shared.next_token += 1;
        self.tokens.insert(token, shared.sequence);
        Ok(token)
    }
    fn poll(&mut self, token: u64) -> Result<WatchState, BackendError> {
        self.live()?;
        let sequence = self.shared.borrow().sequence;
        let seen = self
            .tokens
            .get_mut(&token)
            .ok_or(BackendError::Disconnected)?;
        let changed = *seen != sequence;
        *seen = sequence;
        Ok(WatchState {
            sequence,
            changed,
            overflow: false,
            gone: false,
        })
    }
}

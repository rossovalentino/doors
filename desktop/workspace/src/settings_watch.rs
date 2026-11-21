//! The typed settings registry in a desktop process (spec section 62,
//! P14-REGISTRY): every persisted setting is read through
//! `door_settings_registry`, damaged values read as their defaults (written
//! back and reported once with a notification), changes made by another
//! process are observed live over the storage settings watch, and Settings
//! commits its changes through the same registry. Each event is a runner
//! diagnostic line (`DOOR_P14 SETTINGS commit|observed|reset ...`).
use alloc::format;
use door_service_client::{Error, Notifications, Storage};
use door_service_protocol::Code;
use door_settings_registry::{BackendError, Caller, Error as RegistryError, Registry};

/// How often a process re-reads the storage settings watch: once a second.
fn poll_ticks() -> u64 {
    door_sys::query().map_or(1000, |q| q.tick_hz.max(1))
}

pub struct Watch {
    registry: Registry,
    /// The service registry handle (door.notifications for a reset).
    services: u64,
    next_poll: u64,
}
impl Watch {
    /// Read every setting (repairing damaged ones) and subscribe to changes.
    pub fn load(storage: &mut Storage, services: u64) -> Self {
        let mut watch = Self {
            registry: Registry::new(Caller::User),
            services,
            next_poll: 0,
        };
        let _ = watch.registry.load(storage);
        watch.report();
        watch
    }
    /// Observe changes made elsewhere, at most once a second.
    pub fn poll(&mut self, storage: &mut Storage, now: u64) {
        if now < self.next_poll {
            return;
        }
        self.next_poll = now.saturating_add(poll_ticks());
        if self.registry.poll(storage).is_err() {
            self.registry.disconnected();
        }
        self.report();
    }
    /// The storage connection was replaced: subscribe again on the next poll.
    pub fn disconnected(&mut self) {
        self.registry.disconnected();
        self.next_poll = 0;
    }
    /// Commit `value` to the property `key` (validated against the schema,
    /// rebased on the stored word, persisted and flushed by the backend).
    pub fn commit(&mut self, storage: &mut Storage, key: &str, value: u32) -> Result<(), Error> {
        let mut result = self.try_commit(storage, key, value);
        // The property changed in another process since the last read: the
        // user's choice still wins, proposed again on the fresh value.
        if result == Err(RegistryError::Stale) {
            let _ = self.registry.refresh(storage);
            result = self.try_commit(storage, key, value);
        }
        self.report();
        result.map_err(error)
    }
    fn try_commit(
        &mut self,
        storage: &mut Storage,
        key: &str,
        value: u32,
    ) -> Result<(), RegistryError> {
        let proposal = self.registry.propose_key(key, value)?;
        self.registry.commit(storage, proposal).map(|_| ())
    }
    /// Write the queued events and post one notification for a reset.
    fn report(&mut self) {
        self.registry.drain_events(|event| {
            let _ = door_sys::write(format!("{event}\n").as_bytes());
        });
        let Some(notice) = self.registry.notice() else {
            return;
        };
        let title = notice
            .reset
            .iter()
            .next()
            .map_or("Settings", |id| self.registry.property(id).title);
        let body = format!("{}: {title}.", notice.message());
        if let Ok(mut n) = Notifications::connect(self.services) {
            let _ = n.post(&door_service_client::notifications::Toast {
                icon: door_service_protocol::shell::icon::SETTINGS,
                flags: 0,
                title: "Settings",
                body: &body,
                actions: ["", ""],
            });
        }
        self.registry.dismiss_notice();
    }
}
/// A registry refusal as the service error the app reports.
fn error(e: RegistryError) -> Error {
    Error::Service(match e {
        RegistryError::Backend(BackendError::Denied) => Code::Denied,
        RegistryError::Backend(BackendError::Invalid) => Code::InvalidRequest,
        RegistryError::Backend(BackendError::Corrupt) => Code::Corrupt,
        RegistryError::Backend(BackendError::NotFound) => Code::NotFound,
        RegistryError::Backend(BackendError::Io) => Code::Io,
        RegistryError::Backend(BackendError::Disconnected) => Code::StaleInstance,
        _ => Code::InvalidRequest,
    })
}

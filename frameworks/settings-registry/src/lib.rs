#![no_std]
//! Typed settings registry (spec §62).
//!
//! Every persisted preference is a typed [`Property`]: one bit field of a
//! storage word (a storage-service `Setting` or a registry dword) with its
//! stable key, type, default, scope, write authority, dependencies, apply
//! timing, Settings page and search keywords. Persisted words and their
//! encodings are exactly the ones storage-core validates today.
//!
//! A [`Registry`] separates the steps: read ([`Registry::get`],
//! [`Registry::state`]), propose and validate ([`Registry::propose`],
//! [`Registry::validate`]), commit through a [`Backend`]
//! ([`Registry::commit`]) and observe (in-process [`Registry::subscribe`],
//! cross-process [`Registry::poll`] over the storage settings watch).
//! Damaged values read as defaults with a [`Notice`]; diagnostics are
//! queued [`Event`]s. Allocation-free.
mod backend;
mod event;
mod registry;
pub mod schema;
mod table;

pub use backend::{Backend, BackendError, WatchState};
pub use event::{Event, EventKind, Notice, ResetReason};
pub use registry::{
    Committed, EVENT_CAPACITY, Error, Hit, Invalid, MAX_SUBSCRIBERS, PropertyState, Proposal,
    ReadOnly, Registry, Subscription,
};
pub use schema::{
    Apply, Authority, Caller, Catalog, Coupling, Dependency, Kind, Property, PropertyId,
    PropertySet, Schema, SchemaError, Scope, Store, Visibility, Word,
};
pub use table::{BUILTIN, key};

//! Initial bundle callback transaction. The executor owns mappings and invokes
//! callbacks after releasing its locks; no application code runs inside this model.
//! This is not a dynamic LoadLibrary registry or a Windows loader-lock emulation.
use crate::{BundlePlan, MAX_MODULES, UserAddress};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Attaching,
    Live,
    Detaching,
    Failed,
    Unloaded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    ProcessAttach,
    ProcessDetach,
}

/// Opaque completion identity. A stale/duplicated result cannot advance state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Callback {
    sequence: u32,
    pub module: usize,
    pub base: UserAddress,
    pub entry: UserAddress,
    pub reason: Reason,
    /// Static process attach or orderly process termination uses non-NULL.
    /// Failed-load cleanup uses NULL. The executor supplies a suitable pointer.
    pub reserved_nonnull: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    CallbackPending,
    InvalidCompletion,
    NotLive,
    InvalidInventory,
}

/// One transaction per immutable mapped bundle. The EXE is never called as a DLL.
/// A FALSE attach result receives detach before earlier modules are undone.
/// An exception during attach does not notify the faulting module again.
/// Process detach ignores the DLL's return value, as required by DllMain.
pub struct Lifecycle {
    order: [usize; MAX_MODULES],
    bases: [UserAddress; MAX_MODULES],
    entries: [Option<UserAddress>; MAX_MODULES],
    count: usize,
    cursor: usize,
    attached: usize,
    pending: Option<Callback>,
    sequence: u32,
    phase: Phase,
    failure: Option<usize>,
}

impl Lifecycle {
    /// Reconstruct the callback transaction from a trusted dependency-ordered
    /// startup inventory. The caller must separately verify executable mappings.
    /// Addresses are metadata here and never dereferenced by the planner.
    pub fn from_inventory(modules: &[(UserAddress, Option<UserAddress>)]) -> Result<Self, Error> {
        if modules.len() >= MAX_MODULES {
            return Err(Error::InvalidInventory);
        }
        let mut value = Self::empty();
        for (position, &(base, entry)) in modules.iter().enumerate() {
            let valid = |address: UserAddress| (0x10000..0x8000_0000_0000).contains(&address.0);
            if !valid(base)
                || entry.is_some_and(|address| !valid(address))
                || modules[..position]
                    .iter()
                    .any(|previous| previous.0 == base)
            {
                return Err(Error::InvalidInventory);
            }
            let index = position + 1;
            value.order[position] = index;
            value.bases[index] = base;
            value.entries[index] = entry;
            value.count += 1;
        }
        Ok(value)
    }
    pub fn new(plan: &BundlePlan<'_>) -> Self {
        let mut value = Self::empty();
        for &index in plan.dependency_order() {
            if index == 0 {
                continue;
            }
            let module = plan.module(index).expect("validated dependency index");
            value.order[value.count] = index;
            value.bases[index] = UserAddress(module.source.base.0);
            value.entries[index] = module.entry();
            value.count += 1;
        }
        value
    }
    fn empty() -> Self {
        Self {
            order: [0; MAX_MODULES],
            bases: [UserAddress(0); MAX_MODULES],
            entries: [None; MAX_MODULES],
            count: 0,
            cursor: 0,
            attached: 0,
            pending: None,
            sequence: 0,
            phase: Phase::Attaching,
            failure: None,
        }
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn failed_module(&self) -> Option<usize> {
        self.failure
    }
    /// May mappings be released? Live/no-callback alone is insufficient.
    pub fn may_unmap(&self) -> bool {
        self.pending.is_none() && matches!(self.phase, Phase::Failed | Phase::Unloaded)
    }
    pub fn begin_detach(&mut self) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::CallbackPending);
        }
        if self.phase != Phase::Live {
            return Err(Error::NotLive);
        }
        self.phase = Phase::Detaching;
        Ok(())
    }
    /// Obtain a callback lease, then release any caller locks before invocation.
    /// Reentrant progress while that lease is outstanding explicitly fails.
    pub fn next_callback(&mut self) -> Result<Option<Callback>, Error> {
        if self.pending.is_some() {
            return Err(Error::CallbackPending);
        }
        loop {
            let (position, reason) = match self.phase {
                Phase::Attaching if self.cursor < self.count => {
                    (self.cursor, Reason::ProcessAttach)
                }
                Phase::Attaching => {
                    self.phase = Phase::Live;
                    return Ok(None);
                }
                Phase::Detaching if self.attached != 0 => {
                    (self.attached - 1, Reason::ProcessDetach)
                }
                Phase::Detaching => {
                    self.phase = if self.failure.is_some() {
                        Phase::Failed
                    } else {
                        Phase::Unloaded
                    };
                    return Ok(None);
                }
                _ => return Ok(None),
            };
            let module = self.order[position];
            if let Some(entry) = self.entries[module] {
                // At most two callbacks per module in this non-reusable transaction.
                self.sequence += 1;
                let callback = Callback {
                    sequence: self.sequence,
                    module,
                    base: self.bases[module],
                    entry,
                    reason,
                    reserved_nonnull: self.failure.is_none(),
                };
                self.pending = Some(callback);
                return Ok(Some(callback));
            }
            match reason {
                Reason::ProcessAttach => {
                    self.cursor += 1;
                    self.attached += 1;
                }
                Reason::ProcessDetach => self.attached -= 1,
            }
        }
    }
    pub fn complete(&mut self, callback: Callback, success: bool) -> Result<(), Error> {
        self.finish(callback, success, false)
    }
    /// Records a contained callback exception. The executor must contain the
    /// foreign fault before reporting it; this does not perform unwinding.
    pub fn complete_fault(&mut self, callback: Callback) -> Result<(), Error> {
        self.finish(callback, false, true)
    }
    fn finish(&mut self, callback: Callback, success: bool, fault: bool) -> Result<(), Error> {
        if self.pending != Some(callback) {
            return Err(Error::InvalidCompletion);
        }
        self.pending = None;
        match callback.reason {
            Reason::ProcessAttach if success => {
                self.cursor += 1;
                self.attached += 1;
            }
            Reason::ProcessAttach => {
                if !fault {
                    self.attached += 1;
                }
                self.failure = Some(callback.module);
                self.phase = Phase::Detaching;
            }
            Reason::ProcessDetach => self.attached -= 1,
        }
        Ok(())
    }
}

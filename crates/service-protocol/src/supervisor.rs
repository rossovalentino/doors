//! Bounded door-init service supervision policy (master spec §64, §139.4).
//!
//! This is pure policy: it never launches, signals or blocks. The runtime feeds
//! it time, readiness and exit observations and executes the returned actions.
//! Readiness is distinct from existence, every launch receives a fresh
//! monotonic instance id, and stale observations are rejected by instance.

pub const MAX_SERVICES: usize = 12;
pub const MAX_DEPENDENCIES: usize = 4;
pub const MAX_ACTIONS: usize = MAX_SERVICES * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dependency {
    pub service: &'static str,
    pub optional: bool,
}

/// Restarts use `base << (failures - 1)` ticks capped at `max`. More than
/// `limit` consecutive failures is terminal. A run that stays ready for
/// `stable` ticks clears the consecutive failure count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartPolicy {
    pub limit: u32,
    pub base: u64,
    pub max: u64,
    pub stable: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub name: &'static str,
    pub image: &'static str,
    pub account: u32,
    pub dependencies: &'static [Dependency],
    pub ready_deadline: u64,
    pub stop_deadline: u64,
    pub restart: RestartPolicy,
    pub essential: bool,
    pub autostart: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Capacity,
    Duplicate,
    UnknownDependency,
    SelfDependency,
    Cycle,
    InvalidManifest,
    UnknownService,
    StaleInstance,
    InvalidState,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCause {
    Request,
    Restart,
    Dependency,
    Shutdown,
    Hang,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailReason {
    RestartLimit,
    Dependency,
    CounterExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Stopped,
    Starting {
        instance: u64,
        since: u64,
    },
    Ready {
        instance: u64,
        since: u64,
    },
    Stopping {
        instance: u64,
        deadline: u64,
        cause: StopCause,
        killed: bool,
    },
    Backoff {
        until: u64,
    },
    Failed(FailReason),
}
impl State {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting { .. } => "starting",
            Self::Ready { .. } => "ready",
            Self::Stopping { .. } => "stopping",
            Self::Backoff { .. } => "backoff",
            Self::Failed(FailReason::RestartLimit) => "failed",
            Self::Failed(FailReason::Dependency) => "dep-failed",
            Self::Failed(FailReason::CounterExhausted) => "counter-exhausted",
        }
    }
    pub const fn instance(self) -> Option<u64> {
        match self {
            Self::Starting { instance, .. }
            | Self::Ready { instance, .. }
            | Self::Stopping { instance, .. } => Some(instance),
            _ => None,
        }
    }
    const fn running(self) -> bool {
        matches!(
            self,
            Self::Starting { .. } | Self::Ready { .. } | Self::Stopping { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Launch { service: usize, instance: u64 },
    Stop { service: usize, instance: u64 },
    Kill { service: usize, instance: u64 },
    Fatal { service: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct Actions {
    items: [Option<Action>; MAX_ACTIONS],
    len: usize,
}
impl Actions {
    const fn new() -> Self {
        Self {
            items: [None; MAX_ACTIONS],
            len: 0,
        }
    }
    fn push(&mut self, action: Action) {
        // Each service emits at most two actions per step; capacity is exact.
        self.items[self.len] = Some(action);
        self.len += 1;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn iter(&self) -> impl Iterator<Item = Action> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub name: &'static str,
    pub account: u32,
    pub state: State,
    pub enabled: bool,
    pub degraded: bool,
    pub launches: u64,
    pub failures: u32,
    pub essential: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    manifest: Manifest,
    dependencies: [Option<(usize, bool)>; MAX_DEPENDENCIES],
    state: State,
    enabled: bool,
    failures: u32,
    launches: u64,
    fatal_reported: bool,
}

pub struct Supervisor {
    entries: [Option<Entry>; MAX_SERVICES],
    count: usize,
    order: [usize; MAX_SERVICES],
    next_instance: u64,
    shutting_down: bool,
}

impl Supervisor {
    /// Validates names, dependency references and acyclicity, and fixes one
    /// deterministic topological start order (reverse is the stop order).
    pub fn new(manifests: &[Manifest]) -> Result<Self, Error> {
        if manifests.len() > MAX_SERVICES {
            return Err(Error::Capacity);
        }
        let mut entries: [Option<Entry>; MAX_SERVICES] = [None; MAX_SERVICES];
        for (i, manifest) in manifests.iter().enumerate() {
            if manifest.name.is_empty()
                || manifest.image.is_empty()
                || manifest.ready_deadline == 0
                || manifest.stop_deadline == 0
                || manifest.restart.base == 0
                || manifest.restart.max < manifest.restart.base
                || manifest.restart.limit > 16
            {
                return Err(Error::InvalidManifest);
            }
            if manifests[..i].iter().any(|m| m.name == manifest.name) {
                return Err(Error::Duplicate);
            }
            if manifest.dependencies.len() > MAX_DEPENDENCIES {
                return Err(Error::Capacity);
            }
            let mut dependencies = [None; MAX_DEPENDENCIES];
            for (d, dependency) in manifest.dependencies.iter().enumerate() {
                if dependency.service == manifest.name {
                    return Err(Error::SelfDependency);
                }
                let target = manifests
                    .iter()
                    .position(|m| m.name == dependency.service)
                    .ok_or(Error::UnknownDependency)?;
                if dependencies[..d]
                    .iter()
                    .flatten()
                    .any(|(t, _)| *t == target)
                {
                    return Err(Error::Duplicate);
                }
                dependencies[d] = Some((target, dependency.optional));
            }
            entries[i] = Some(Entry {
                manifest: *manifest,
                dependencies,
                state: State::Stopped,
                enabled: manifest.autostart,
                failures: 0,
                launches: 0,
                fatal_reported: false,
            });
        }
        // Kahn's algorithm over dependency -> dependent edges.
        let count = manifests.len();
        let mut pending = [0usize; MAX_SERVICES];
        for (i, slot) in entries[..count].iter().enumerate() {
            pending[i] = slot
                .as_ref()
                .map_or(0, |e| e.dependencies.iter().flatten().count());
        }
        let mut order = [0usize; MAX_SERVICES];
        let mut placed = [false; MAX_SERVICES];
        for position in order.iter_mut().take(count) {
            let next = (0..count)
                .find(|&i| !placed[i] && pending[i] == 0)
                .ok_or(Error::Cycle)?;
            placed[next] = true;
            *position = next;
            for (i, slot) in entries[..count].iter().enumerate() {
                let depends = slot
                    .as_ref()
                    .is_some_and(|e| e.dependencies.iter().flatten().any(|(t, _)| *t == next));
                if depends {
                    pending[i] -= 1;
                }
            }
        }
        Ok(Self {
            entries,
            count,
            order,
            next_instance: 1,
            shutting_down: false,
        })
    }

    pub const fn len(&self) -> usize {
        self.count
    }
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        (0..self.count).find(|&i| self.entry(i).manifest.name == name)
    }
    pub fn manifest(&self, service: usize) -> Option<&Manifest> {
        self.entries.get(service)?.as_ref().map(|e| &e.manifest)
    }
    pub fn state(&self, service: usize) -> Option<State> {
        self.entries.get(service)?.as_ref().map(|e| e.state)
    }
    pub fn start_order(&self) -> &[usize] {
        &self.order[..self.count]
    }
    pub const fn shutting_down(&self) -> bool {
        self.shutting_down
    }
    pub fn snapshot(&self, service: usize) -> Option<Snapshot> {
        let entry = self.entries.get(service)?.as_ref()?;
        Some(Snapshot {
            name: entry.manifest.name,
            account: entry.manifest.account,
            state: entry.state,
            enabled: entry.enabled,
            degraded: self.degraded(service),
            launches: entry.launches,
            failures: entry.failures,
            essential: entry.manifest.essential,
        })
    }
    /// Ready, but at least one optional dependency is not ready.
    pub fn degraded(&self, service: usize) -> bool {
        let Some(entry) = self.entries.get(service).and_then(Option::as_ref) else {
            return false;
        };
        matches!(entry.state, State::Ready { .. })
            && entry
                .dependencies
                .iter()
                .flatten()
                .any(|&(d, optional)| optional && !self.is_ready(d))
    }
    /// True once every service has no running instance.
    pub fn quiescent(&self) -> bool {
        (0..self.count).all(|i| !self.entry(i).state.running())
    }

    fn entry(&self, i: usize) -> &Entry {
        self.entries[i].as_ref().expect("validated service")
    }
    fn entry_mut(&mut self, i: usize) -> &mut Entry {
        self.entries[i].as_mut().expect("validated service")
    }
    fn is_ready(&self, i: usize) -> bool {
        matches!(self.entry(i).state, State::Ready { .. })
    }
    fn check(&self, service: usize) -> Result<(), Error> {
        if service < self.count {
            Ok(())
        } else {
            Err(Error::UnknownService)
        }
    }

    /// Readiness report from the instance itself.
    pub fn ready(&mut self, service: usize, instance: u64, now: u64) -> Result<(), Error> {
        self.check(service)?;
        let entry = self.entry_mut(service);
        match entry.state {
            State::Starting { instance: i, .. } if i == instance => {
                entry.state = State::Ready {
                    instance,
                    since: now,
                };
                Ok(())
            }
            state if state.instance() == Some(instance) => Err(Error::InvalidState),
            _ => Err(Error::StaleInstance),
        }
    }

    /// Exit (or completed forced kill) of one instance.
    pub fn exited(&mut self, service: usize, instance: u64, now: u64) -> Result<(), Error> {
        self.check(service)?;
        let shutting_down = self.shutting_down;
        let entry = self.entry_mut(service);
        if entry.state.instance() != Some(instance) {
            return Err(Error::StaleInstance);
        }
        let failed = match entry.state {
            State::Stopping { cause, .. } => cause == StopCause::Hang,
            State::Ready { since, .. } => {
                if now.saturating_sub(since) >= entry.manifest.restart.stable {
                    entry.failures = 0;
                }
                true
            }
            _ => true,
        };
        if !failed || shutting_down {
            entry.state = State::Stopped;
            return Ok(());
        }
        entry.failures = entry.failures.saturating_add(1);
        let policy = entry.manifest.restart;
        entry.state = if entry.failures > policy.limit {
            State::Failed(FailReason::RestartLimit)
        } else {
            let shift = (entry.failures - 1).min(63);
            let delay = policy
                .base
                .checked_shl(shift)
                .filter(|d| d >> shift == policy.base)
                .map_or(policy.max, |d| d.min(policy.max));
            State::Backoff {
                until: now.saturating_add(delay),
            }
        };
        Ok(())
    }

    /// Administrative enable; clears a terminal failure.
    pub fn start(&mut self, service: usize) -> Result<(), Error> {
        self.check(service)?;
        if self.shutting_down {
            return Err(Error::ShuttingDown);
        }
        let entry = self.entry_mut(service);
        entry.enabled = true;
        if matches!(entry.state, State::Failed(_)) {
            entry.state = State::Stopped;
            entry.failures = 0;
            entry.fatal_reported = false;
        }
        Ok(())
    }

    /// Administrative disable with graceful stop and forced-kill deadline.
    pub fn stop(&mut self, service: usize, now: u64) -> Result<Option<Action>, Error> {
        self.check(service)?;
        let entry = self.entry_mut(service);
        entry.enabled = false;
        Ok(self.request_stop(service, now, StopCause::Request))
    }

    /// Administrative restart: graceful stop, then a fresh instance.
    pub fn restart(&mut self, service: usize, now: u64) -> Result<Option<Action>, Error> {
        self.start(service)?;
        self.entry_mut(service).failures = 0;
        if let State::Backoff { .. } = self.entry(service).state {
            self.entry_mut(service).state = State::Stopped;
        }
        Ok(self.request_stop(service, now, StopCause::Restart))
    }

    fn request_stop(&mut self, service: usize, now: u64, cause: StopCause) -> Option<Action> {
        let entry = self.entry_mut(service);
        match entry.state {
            State::Starting { instance, .. } | State::Ready { instance, .. } => {
                entry.state = State::Stopping {
                    instance,
                    deadline: now.saturating_add(entry.manifest.stop_deadline),
                    cause,
                    killed: false,
                };
                Some(Action::Stop { service, instance })
            }
            State::Backoff { .. } if cause != StopCause::Restart => {
                entry.state = State::Stopped;
                None
            }
            _ => None,
        }
    }

    pub fn begin_shutdown(&mut self) {
        self.shutting_down = true;
        for i in 0..self.count {
            let entry = self.entry_mut(i);
            entry.enabled = false;
            if let State::Backoff { .. } = entry.state {
                entry.state = State::Stopped;
            }
        }
    }

    fn dependents_quiet(&self, service: usize) -> bool {
        (0..self.count).all(|i| {
            let entry = self.entry(i);
            !entry.state.running()
                || !entry
                    .dependencies
                    .iter()
                    .flatten()
                    .any(|&(d, _)| d == service)
        })
    }

    /// Advance policy to `now` and return the actions the runtime must take.
    pub fn step(&mut self, now: u64) -> Actions {
        let mut actions = Actions::new();
        // Deadlines: hung readiness and overdue graceful stops are killed.
        for i in 0..self.count {
            let entry = self.entry_mut(i);
            match entry.state {
                State::Starting { instance, since }
                    if now.saturating_sub(since) >= entry.manifest.ready_deadline =>
                {
                    entry.state = State::Stopping {
                        instance,
                        deadline: now,
                        cause: StopCause::Hang,
                        killed: true,
                    };
                    actions.push(Action::Kill {
                        service: i,
                        instance,
                    });
                }
                State::Stopping {
                    instance,
                    deadline,
                    cause,
                    killed: false,
                } if now >= deadline => {
                    entry.state = State::Stopping {
                        instance,
                        deadline,
                        cause,
                        killed: true,
                    };
                    actions.push(Action::Kill {
                        service: i,
                        instance,
                    });
                }
                _ => {}
            }
        }
        if self.shutting_down {
            for position in (0..self.count).rev() {
                let i = self.order[position];
                if matches!(
                    self.entry(i).state,
                    State::Starting { .. } | State::Ready { .. }
                ) && self.dependents_quiet(i)
                    && let Some(action) = self.request_stop(i, now, StopCause::Shutdown)
                {
                    actions.push(action);
                }
            }
            return actions;
        }
        for position in 0..self.count {
            let i = self.order[position];
            let entry = *self.entry(i);
            let required = entry.dependencies.iter().flatten().filter(|(_, o)| !o);
            let mut required_ready = true;
            let mut required_failed = false;
            for &(d, _) in required {
                required_ready &= self.is_ready(d);
                required_failed |= matches!(self.entry(d).state, State::Failed(_));
            }
            let optional_settled =
                entry
                    .dependencies
                    .iter()
                    .flatten()
                    .filter(|(_, o)| *o)
                    .all(|&(d, _)| {
                        let dep = self.entry(d);
                        !dep.enabled || matches!(dep.state, State::Ready { .. } | State::Failed(_))
                    });
            match entry.state {
                State::Starting { .. } | State::Ready { .. } if !required_ready => {
                    if let Some(action) = self.request_stop(i, now, StopCause::Dependency) {
                        actions.push(action);
                    }
                }
                State::Stopped if required_failed && entry.enabled => {
                    self.entry_mut(i).state = State::Failed(FailReason::Dependency);
                }
                State::Failed(FailReason::Dependency) if !required_failed => {
                    self.entry_mut(i).state = State::Stopped;
                }
                State::Backoff { until } if now >= until => {
                    self.entry_mut(i).state = State::Stopped;
                }
                _ => {}
            }
            let entry = self.entry_mut(i);
            if entry.state == State::Stopped && entry.enabled && required_ready && optional_settled
            {
                let instance = self.next_instance;
                let Some(next_instance) = self.next_instance.checked_add(1) else {
                    self.entry_mut(i).state = State::Failed(FailReason::CounterExhausted);
                    continue;
                };
                let Some(launches) = self.entry(i).launches.checked_add(1) else {
                    self.entry_mut(i).state = State::Failed(FailReason::CounterExhausted);
                    continue;
                };
                self.next_instance = next_instance;
                let entry = self.entry_mut(i);
                entry.state = State::Starting {
                    instance,
                    since: now,
                };
                entry.launches = launches;
                actions.push(Action::Launch {
                    service: i,
                    instance,
                });
            }
            let entry = self.entry_mut(i);
            if matches!(entry.state, State::Failed(_))
                && entry.manifest.essential
                && !entry.fatal_reported
            {
                entry.fatal_reported = true;
                actions.push(Action::Fatal { service: i });
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_instance_and_launch_counters_never_publish_aliases() {
        let mut s = Supervisor::new(&[manifest("a", &[])]).unwrap();
        s.next_instance = u64::MAX;
        assert!(s.step(0).is_empty());
        assert_eq!(
            s.state(0),
            Some(State::Failed(FailReason::CounterExhausted))
        );
        let mut s = Supervisor::new(&[manifest("a", &[])]).unwrap();
        s.entry_mut(0).launches = u64::MAX;
        assert!(s.step(0).is_empty());
        assert_eq!(s.next_instance, 1);
        assert_eq!(
            s.state(0),
            Some(State::Failed(FailReason::CounterExhausted))
        );
    }

    const POLICY: RestartPolicy = RestartPolicy {
        limit: 3,
        base: 10,
        max: 40,
        stable: 100,
    };
    const fn manifest(name: &'static str, dependencies: &'static [Dependency]) -> Manifest {
        Manifest {
            name,
            image: "svc.elf",
            account: 100,
            dependencies,
            ready_deadline: 50,
            stop_deadline: 20,
            restart: POLICY,
            essential: false,
            autostart: true,
        }
    }
    const fn req(service: &'static str) -> Dependency {
        Dependency {
            service,
            optional: false,
        }
    }
    const fn opt(service: &'static str) -> Dependency {
        Dependency {
            service,
            optional: true,
        }
    }

    fn launches(actions: &Actions) -> std::vec::Vec<(usize, u64)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Launch { service, instance } => Some((service, instance)),
                _ => None,
            })
            .collect()
    }
    extern crate std;

    fn graph() -> Supervisor {
        static STORAGE: [Dependency; 1] = [req("registry")];
        static SESSION: [Dependency; 2] = [req("storage"), opt("diag")];
        Supervisor::new(&[
            manifest("session", &SESSION),
            manifest("diag", &[]),
            manifest("storage", &STORAGE),
            manifest("registry", &[]),
        ])
        .unwrap()
    }

    #[test]
    fn validation_rejects_bad_graphs() {
        static A: [Dependency; 1] = [req("b")];
        static B: [Dependency; 1] = [req("a")];
        static SELF: [Dependency; 1] = [req("a")];
        static MISSING: [Dependency; 1] = [req("zz")];
        assert_eq!(
            Supervisor::new(&[manifest("a", &A), manifest("b", &B)]).err(),
            Some(Error::Cycle)
        );
        assert_eq!(
            Supervisor::new(&[manifest("a", &SELF)]).err(),
            Some(Error::SelfDependency)
        );
        assert_eq!(
            Supervisor::new(&[manifest("a", &MISSING)]).err(),
            Some(Error::UnknownDependency)
        );
        assert_eq!(
            Supervisor::new(&[manifest("a", &[]), manifest("a", &[])]).err(),
            Some(Error::Duplicate)
        );
        let mut bad = manifest("a", &[]);
        bad.restart.base = 0;
        assert_eq!(Supervisor::new(&[bad]).err(), Some(Error::InvalidManifest));
    }

    #[test]
    fn readiness_gates_dependents_in_topological_order() {
        let mut s = graph();
        let reg = s.find("registry").unwrap();
        let storage = s.find("storage").unwrap();
        let session = s.find("session").unwrap();
        let diag = s.find("diag").unwrap();
        let first = s.step(0);
        let started = launches(&first);
        // Existence is not readiness: storage waits for registry Ready.
        assert!(started.iter().any(|&(i, _)| i == reg));
        assert!(started.iter().any(|&(i, _)| i == diag));
        assert!(!started.iter().any(|&(i, _)| i == storage));
        let reg_instance = s.state(reg).unwrap().instance().unwrap();
        assert!(s.step(1).is_empty());
        s.ready(reg, reg_instance, 2).unwrap();
        let next = launches(&s.step(3));
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].0, storage);
        assert!(next[0].1 > reg_instance);
        // session waits for its optional dependency to settle too.
        let st = s.state(storage).unwrap().instance().unwrap();
        s.ready(storage, st, 4).unwrap();
        assert!(launches(&s.step(5)).is_empty());
        let d = s.state(diag).unwrap().instance().unwrap();
        s.ready(diag, d, 6).unwrap();
        assert_eq!(launches(&s.step(7))[0].0, session);
    }

    #[test]
    fn stale_instances_and_double_ready_are_rejected() {
        let mut s = graph();
        let reg = s.find("registry").unwrap();
        s.step(0);
        let i = s.state(reg).unwrap().instance().unwrap();
        assert_eq!(s.ready(reg, i + 100, 1), Err(Error::StaleInstance));
        s.ready(reg, i, 1).unwrap();
        assert_eq!(s.ready(reg, i, 2), Err(Error::InvalidState));
        assert_eq!(s.exited(reg, i + 100, 2), Err(Error::StaleInstance));
        assert_eq!(s.ready(99, i, 2), Err(Error::UnknownService));
    }

    #[test]
    fn hang_is_killed_and_restarted_with_bounded_backoff() {
        let mut s = Supervisor::new(&[manifest("hang", &[])]).unwrap();
        let mut now = 0;
        let mut instances = std::vec::Vec::new();
        let mut delays = std::vec::Vec::new();
        loop {
            let actions = s.step(now);
            for action in actions.iter() {
                match action {
                    Action::Launch { instance, .. } => instances.push(instance),
                    Action::Kill { instance, .. } => {
                        s.exited(0, instance, now).unwrap();
                        if let State::Backoff { until } = s.state(0).unwrap() {
                            delays.push(until - now);
                        }
                    }
                    other => panic!("{other:?}"),
                }
            }
            if matches!(s.state(0).unwrap(), State::Failed(_)) {
                break;
            }
            now += 1;
            assert!(now < 10_000);
        }
        assert_eq!(delays, [10, 20, 40]);
        assert_eq!(instances.len(), 4);
        assert!(instances.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(s.state(0), Some(State::Failed(FailReason::RestartLimit)));
        // Terminal: no further launches until an administrator restarts it.
        assert!(s.step(now + 1000).is_empty());
        s.start(0).unwrap();
        assert_eq!(launches(&s.step(now + 1001)).len(), 1);
    }

    #[test]
    fn stable_runs_reset_restart_loop_accounting() {
        let mut s = Supervisor::new(&[manifest("crash", &[])]).unwrap();
        let mut now = 0;
        for _ in 0..10 {
            let a = launches(&s.step(now));
            let (_, instance) = a[0];
            s.ready(0, instance, now).unwrap();
            now += 150; // longer than `stable`
            s.exited(0, instance, now).unwrap();
            assert_eq!(s.state(0), Some(State::Backoff { until: now + 10 }));
            now += 10;
        }
        assert_eq!(s.snapshot(0).unwrap().failures, 1);
    }

    #[test]
    fn crash_loop_fails_and_essential_failure_is_fatal() {
        let mut m = manifest("core", &[]);
        m.essential = true;
        m.restart.limit = 1;
        let mut s = Supervisor::new(&[m]).unwrap();
        let (_, i1) = launches(&s.step(0))[0];
        s.exited(0, i1, 1).unwrap();
        let (_, i2) = launches(&s.step(11))[0];
        s.exited(0, i2, 12).unwrap();
        assert_eq!(s.state(0), Some(State::Failed(FailReason::RestartLimit)));
        let fatal: std::vec::Vec<_> = s.step(13).iter().collect();
        assert_eq!(fatal, [Action::Fatal { service: 0 }]);
        assert!(s.step(14).is_empty(), "fatal reported once");
    }

    #[test]
    fn optional_failure_degrades_and_required_failure_cascades() {
        let mut s = graph();
        let ids = ["registry", "storage", "diag", "session"].map(|n| s.find(n).unwrap());
        let [reg, storage, diag, session] = ids;
        let mut now = 0;
        // Bring registry and storage up; let diag crash-loop to Failed.
        loop {
            for action in s.step(now).iter() {
                if let Action::Launch { service, instance } = action {
                    if service == diag {
                        s.exited(diag, instance, now).unwrap();
                    } else {
                        s.ready(service, instance, now).unwrap();
                    }
                }
            }
            if s.is_ready(session) {
                break;
            }
            now += 1;
            assert!(now < 10_000);
        }
        assert_eq!(s.state(diag), Some(State::Failed(FailReason::RestartLimit)));
        assert!(s.degraded(session));
        assert!(!s.degraded(storage));
        // Registry fails permanently: storage then session are stopped and
        // marked dependency-failed, never relaunched against a dead registry.
        let mut m = s.entry(reg).manifest;
        m.restart.limit = 0;
        s.entry_mut(reg).manifest = m;
        let r = s.state(reg).unwrap().instance().unwrap();
        s.exited(reg, r, now).unwrap();
        for _ in 0..4 {
            now += 1;
            for action in s.step(now).iter() {
                match action {
                    Action::Stop { service, instance } => s.exited(service, instance, now).unwrap(),
                    Action::Launch { .. } => panic!("launch against failed dependency"),
                    _ => {}
                }
            }
        }
        assert_eq!(
            s.state(storage),
            Some(State::Failed(FailReason::Dependency))
        );
        assert_eq!(
            s.state(session),
            Some(State::Failed(FailReason::Dependency))
        );
        // Recovery of the dependency releases dependents.
        s.start(reg).unwrap();
        let mut up = false;
        for _ in 0..20 {
            now += 1;
            for action in s.step(now).iter() {
                if let Action::Launch { service, instance } = action
                    && service != diag
                {
                    s.ready(service, instance, now).unwrap();
                }
            }
            if s.is_ready(session) {
                up = true;
                break;
            }
        }
        assert!(up);
    }

    #[test]
    fn graceful_stop_then_forced_kill_after_deadline() {
        let mut s = Supervisor::new(&[manifest("a", &[])]).unwrap();
        let (_, i) = launches(&s.step(0))[0];
        s.ready(0, i, 1).unwrap();
        assert_eq!(
            s.stop(0, 5).unwrap(),
            Some(Action::Stop {
                service: 0,
                instance: i
            })
        );
        assert!(s.step(10).is_empty());
        let kill: std::vec::Vec<_> = s.step(25).iter().collect();
        assert_eq!(
            kill,
            [Action::Kill {
                service: 0,
                instance: i
            }]
        );
        s.exited(0, i, 26).unwrap();
        assert_eq!(s.state(0), Some(State::Stopped));
        assert!(s.step(100).is_empty(), "disabled service stays stopped");
        assert_eq!(s.snapshot(0).unwrap().failures, 0);
    }

    #[test]
    fn restart_creates_new_instance_without_failure() {
        let mut s = Supervisor::new(&[manifest("a", &[])]).unwrap();
        let (_, i) = launches(&s.step(0))[0];
        s.ready(0, i, 1).unwrap();
        s.restart(0, 2).unwrap();
        s.exited(0, i, 3).unwrap();
        let (_, j) = launches(&s.step(4))[0];
        assert!(j > i);
        assert_eq!(s.snapshot(0).unwrap().launches, 2);
        assert_eq!(s.snapshot(0).unwrap().failures, 0);
    }

    #[test]
    fn shutdown_stops_in_reverse_dependency_order() {
        let mut s = graph();
        let mut now = 0;
        while !s.is_ready(s.find("session").unwrap()) {
            for action in s.step(now).iter() {
                if let Action::Launch { service, instance } = action {
                    s.ready(service, instance, now).unwrap();
                }
            }
            now += 1;
        }
        s.begin_shutdown();
        let mut stopped = std::vec::Vec::new();
        while !s.quiescent() {
            now += 1;
            let actions = s.step(now);
            for action in actions.iter() {
                match action {
                    Action::Stop { service, instance } => {
                        stopped.push(s.manifest(service).unwrap().name);
                        s.exited(service, instance, now).unwrap();
                    }
                    Action::Launch { .. } => panic!("launch during shutdown"),
                    _ => {}
                }
            }
            assert!(now < 100);
        }
        let pos = |n| stopped.iter().position(|&s| s == n).unwrap();
        assert!(pos("session") < pos("storage"));
        assert!(pos("storage") < pos("registry"));
        assert!(pos("session") < pos("diag"));
        assert_eq!(s.start(0), Err(Error::ShuttingDown));
    }

    #[test]
    fn shutdown_kills_service_ignoring_stop_request() {
        let mut s = Supervisor::new(&[manifest("a", &[])]).unwrap();
        let (_, i) = launches(&s.step(0))[0];
        s.ready(0, i, 1).unwrap();
        s.begin_shutdown();
        let stop: std::vec::Vec<_> = s.step(2).iter().collect();
        assert_eq!(
            stop,
            [Action::Stop {
                service: 0,
                instance: i
            }]
        );
        assert!(s.step(10).is_empty());
        let kill: std::vec::Vec<_> = s.step(22).iter().collect();
        assert_eq!(
            kill,
            [Action::Kill {
                service: 0,
                instance: i
            }]
        );
        s.exited(0, i, 23).unwrap();
        assert!(s.quiescent());
    }

    #[test]
    fn required_dependency_restart_bounces_dependents() {
        static B: [Dependency; 1] = [req("a")];
        let mut s = Supervisor::new(&[manifest("a", &[]), manifest("b", &B)]).unwrap();
        let mut now = 0;
        while !s.is_ready(1) {
            for action in s.step(now).iter() {
                if let Action::Launch { service, instance } = action {
                    s.ready(service, instance, now).unwrap();
                }
            }
            now += 1;
        }
        let b = s.state(1).unwrap().instance().unwrap();
        s.restart(0, now).unwrap();
        let stops: std::vec::Vec<_> = s.step(now).iter().collect();
        assert!(stops.contains(&Action::Stop {
            service: 1,
            instance: b
        }));
        assert_eq!(s.snapshot(1).unwrap().failures, 0);
    }
}

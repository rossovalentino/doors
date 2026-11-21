//! Import closure of one executable over a fixed, trusted provider catalog.
//!
//! The kernel no longer maps a fixed DLL set for ordinary applications: it
//! maps exactly the executable's import, delay-import and forwarder closure.
//! Names resolve only against the catalog (after the explicit API-set table);
//! there is no search path. This pass only selects modules; the selected bundle
//! is then validated completely by `BundlePlan::with_policy`.
use crate::layout::has_extension;
use crate::{
    Error, Limit, LoadPolicy, MAX_MODULES, ModulePlan, ModuleSource, Reason, api_set_host, dword,
};

/// Catalog capacity accepted by the closure walk.
pub const MAX_PROVIDERS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Closure {
    modules: [usize; MAX_MODULES],
    count: usize,
}
impl Closure {
    /// Selected catalog indices: required providers first (in the given
    /// order), then the remaining selection in catalog order.
    pub fn providers(&self) -> &[usize] {
        &self.modules[..self.count]
    }
}

/// Errors name the executable as module 0 and catalog entry `i` as `1 + i`.
pub fn import_closure(
    executable: ModuleSource<'_>,
    providers: &[ModuleSource<'_>],
    required: &[usize],
    policy: LoadPolicy,
) -> Result<Closure, Error> {
    if providers.len() > MAX_PROVIDERS {
        return Err(Error::new(0, None, Reason::ResourceLimit(Limit::Modules)));
    }
    let mut selected = 0u64;
    for &index in required {
        if index >= providers.len() {
            return Err(Error::new(0, None, Reason::MissingModule));
        }
        selected |= 1 << index;
    }
    let mut scanned = 0u64;
    scan(executable, 0, providers, policy, &mut selected)?;
    // Each round scans at least one new provider, so this terminates within
    // the catalog size; the selection mask only grows.
    while selected & !scanned != 0 {
        let index = (selected & !scanned).trailing_zeros() as usize;
        scanned |= 1 << index;
        scan(
            providers[index],
            1 + index,
            providers,
            policy,
            &mut selected,
        )?;
        if selected.count_ones() as usize + 1 > policy.limits().modules {
            return Err(Error::new(
                1 + index,
                None,
                Reason::ResourceLimit(Limit::Modules),
            ));
        }
    }
    let mut closure = Closure {
        modules: [0; MAX_MODULES],
        count: 0,
    };
    if selected.count_ones() as usize + 1 > policy.limits().modules {
        return Err(Error::new(0, None, Reason::ResourceLimit(Limit::Modules)));
    }
    for &index in required {
        if !closure.providers().contains(&index) {
            closure.modules[closure.count] = index;
            closure.count += 1;
        }
    }
    for index in 0..providers.len() {
        if selected & (1 << index) != 0 && !closure.providers().contains(&index) {
            closure.modules[closure.count] = index;
            closure.count += 1;
        }
    }
    Ok(closure)
}

fn scan(
    source: ModuleSource<'_>,
    index: usize,
    providers: &[ModuleSource<'_>],
    policy: LoadPolicy,
    selected: &mut u64,
) -> Result<(), Error> {
    let image = crate::parse(source.bytes, policy)
        .map_err(|error| Error::new(index, None, Reason::Image(error)))?;
    let module = ModulePlan {
        source,
        index,
        image,
        policy,
    };
    let mut select = |name: &str, forwarder: bool| -> Result<(), Error> {
        let provider = find(providers, name, forwarder, policy)
            .ok_or_else(|| module.error(0, Reason::MissingModule))?;
        *selected |= 1 << provider;
        Ok(())
    };
    for import in 0..module.import_count()? {
        let name = module
            .image
            .import_name(import)
            .map_err(|error| module.error(0, Reason::Image(error)))?;
        select(name, false)?;
    }
    for delay in 0..module.delay_count()? {
        select(module.delay_import(delay)?.name, false)?;
    }
    let (directory, _) = module.directory(0)?;
    if directory != 0 {
        let table = module.read(directory, 40)?;
        let functions = dword(table, 20);
        // Same ordinal-space cap as the bundle plan, enforced before the
        // walk so a hostile count cannot drive an unbounded scan.
        if dword(table, 16)
            .checked_add(functions)
            .is_none_or(|end| end > 65536)
        {
            return Err(module.error(directory, Reason::InvalidExport));
        }
        for function in 0..functions {
            let rva = dword(module.read(dword(table, 28) + function * 4, 4)?, 0);
            if let Some((name, _)) = module.forwarder(rva)? {
                // A module forwarding to itself needs no catalog entry.
                let own = source
                    .name
                    .get(..source.name.len().saturating_sub(4))
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(name));
                if !own && !name.eq_ignore_ascii_case(source.name) {
                    select(name, true)?;
                }
            }
        }
    }
    Ok(())
}

fn find(
    providers: &[ModuleSource<'_>],
    name: &str,
    forwarder: bool,
    policy: LoadPolicy,
) -> Option<usize> {
    let name = if policy != LoadPolicy::Initial {
        api_set_host(name).unwrap_or(name)
    } else {
        name
    };
    providers.iter().position(|provider| {
        provider.name.eq_ignore_ascii_case(name)
            || (forwarder
                && !has_extension(name, ".dll")
                && provider
                    .name
                    .get(..provider.name.len().saturating_sub(4))
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(name)))
    })
}

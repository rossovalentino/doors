use core::convert::Infallible;

use crate::layout::{has_extension, infallible_plan_error, valid_module_name};
use crate::{
    BundlePlan, Error, Feature, Fixup, FixupKind, Limit, LoadPolicy, MAX_DELAY_DESCRIPTORS,
    MAX_IMPORT_DESCRIPTORS, Machine, ModulePlan, Reason, Rva, Symbol, UserAddress, VisitError,
    dword, word,
};

#[derive(Clone, Copy)]
pub(crate) struct Import<'a> {
    pub name: &'a str,
    pub lookup: u32,
    pub iat: u32,
    pub count: usize,
}
#[derive(Clone, Copy)]
enum Export<'a> {
    Address(u32),
    Forwarder { module: usize, symbol: Symbol<'a> },
}

impl<'a> BundlePlan<'a> {
    pub(crate) fn validate_dependencies(&mut self) -> Result<(), Error> {
        for index in 0..self.count {
            let module = ModulePlan {
                source: self.get(index).source,
                index,
                image: self.get(index).image,
                policy: self.policy,
            };
            let count = module.import_count()?;
            for import_index in 0..count {
                let import = module.import(import_index)?;
                let dependency = self
                    .find_module(index, import.name, false)
                    .map_err(|error| module.error(import.lookup, error.reason))?;
                self.dependencies[index] |= 1u64 << dependency;
            }
            // Delay imports are bound eagerly but impose no initialization
            // edge: Windows permits delay-load cycles, so they never order.
            for delay_index in 0..module.delay_count()? {
                let import = module.delay_import(delay_index)?;
                let dependency = self
                    .find_module(index, import.name, false)
                    .map_err(|error| module.error(import.descriptor, error.reason))?;
                self.delay_dependencies[index] |= 1u64 << dependency;
            }
            let (directory, _) = module.directory(0)?;
            if directory == 0 {
                continue;
            }
            let table = module.read(directory, 40)?;
            let base = dword(table, 16);
            let functions = dword(table, 20);
            if base.checked_add(functions).is_none_or(|end| end > 65536) {
                return Err(module.error(directory, Reason::InvalidExport));
            }
            let names = module
                .image
                .export_count()
                .map_err(|error| module.error(directory, Reason::Image(error)))?;
            let mut prior_name = None;
            for name_index in 0..names {
                let name = module
                    .image
                    .export_name(name_index)
                    .map_err(|error| module.error(directory, Reason::Image(error)))?;
                if prior_name.is_some_and(|prior: &str| prior >= name) {
                    return Err(module.error(directory, Reason::InvalidExport));
                }
                prior_name = Some(name);
            }
            for function in 0..functions {
                let rva = dword(module.read(dword(table, 28) + function * 4, 4)?, 0);
                // Ordinal holes are valid but can never satisfy an import.
                if rva == 0 {
                    continue;
                }
                match self.export(index, rva)? {
                    Export::Address(_) => {}
                    Export::Forwarder { module: target, .. } if target != index => {
                        self.dependencies[index] |= 1u64 << target;
                    }
                    Export::Forwarder { .. } => {}
                }
            }
        }
        let mut visiting = 0;
        let mut complete = 0;
        let mut output = 0;
        for index in 0..self.count {
            self.order_dependency(index, &mut visiting, &mut complete, &mut output)?;
        }
        // Validate even currently unused forwarders. A supplied module never
        // carries a latent malformed export into the process image.
        for index in 0..self.count {
            let module = self.get(index);
            let (directory, _) = module.directory(0)?;
            if directory == 0 {
                continue;
            }
            let table = module.read(directory, 40)?;
            for function in 0..dword(table, 20) {
                let rva = dword(module.read(dword(table, 28) + function * 4, 4)?, 0);
                if rva != 0 {
                    self.resolve_export(
                        index,
                        Symbol::Ordinal((dword(table, 16) + function) as u16),
                    )?;
                }
            }
        }
        Ok(())
    }
    fn order_dependency(
        &mut self,
        index: usize,
        visiting: &mut u64,
        complete: &mut u64,
        output: &mut usize,
    ) -> Result<(), Error> {
        let bit = 1u64 << index;
        if *complete & bit != 0 {
            return Ok(());
        }
        if *visiting & bit != 0 {
            return Err(self.get(index).error(0, Reason::DependencyCycle));
        }
        *visiting |= bit;
        for dependency in 0..self.count {
            if self.dependencies[index] & (1u64 << dependency) != 0 {
                self.order_dependency(dependency, visiting, complete, output)?;
            }
        }
        *visiting &= !bit;
        *complete |= bit;
        self.order[*output] = index;
        *output += 1;
        Ok(())
    }
    fn find_module(&self, from: usize, name: &str, forwarder: bool) -> Result<usize, Error> {
        let name = if self.policy != LoadPolicy::Initial {
            crate::api_set_host(name).unwrap_or(name)
        } else {
            name
        };
        if !valid_module_name(name) || (!forwarder && !has_extension(name, ".dll")) {
            return Err(self.get(from).error(0, Reason::InvalidModuleName));
        }
        for index in 1..self.count {
            let candidate = self.get(index).source.name;
            if candidate.eq_ignore_ascii_case(name)
                || (forwarder
                    && !has_extension(name, ".dll")
                    && candidate
                        .get(..candidate.len() - 4)
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(name)))
            {
                return Ok(index);
            }
        }
        Err(self.get(from).error(0, Reason::MissingModule))
    }
    pub(crate) fn resolve(
        &self,
        index: usize,
        symbol: Symbol<'_>,
        seen: &mut [(usize, u32); 16],
        depth: usize,
    ) -> Result<UserAddress, Error> {
        let module = self.get(index);
        if depth == seen.len() {
            return Err(module.error(0, Reason::ResourceLimit(Limit::Forwarders)));
        }
        let rva = module.export_rva(symbol)?;
        if seen[..depth].contains(&(index, rva)) {
            return Err(module.error(rva, Reason::ForwarderCycle));
        }
        seen[depth] = (index, rva);
        match self.export(index, rva)? {
            Export::Address(rva) => Ok(module.address(rva)),
            Export::Forwarder { module, symbol } => self.resolve(module, symbol, seen, depth + 1),
        }
    }
    fn export(&self, index: usize, rva: u32) -> Result<Export<'a>, Error> {
        let module = self.get(index);
        match module.forwarder(rva)? {
            Some((name, symbol)) => Ok(Export::Forwarder {
                module: self.find_module(index, name, true)?,
                symbol,
            }),
            None => {
                // PE exports describe addresses, including zero-filled data. A
                // caller which will invoke the result must additionally require RX.
                module.mapped_section(rva, 1)?;
                Ok(Export::Address(rva))
            }
        }
    }
    pub(crate) fn walk_imports<E>(
        &self,
        index: usize,
        mut visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        let module = self.get(index);
        let width = module.pointer_bytes();
        for descriptor in 0..module.import_count()? {
            let import = module.import(descriptor)?;
            let target = self
                .find_module(index, import.name, false)
                .map_err(|error| module.error(import.lookup, error.reason))?;
            for function in 0..import.count {
                let rva = import.iat + function as u32 * width;
                let value = self
                    .resolve_export(
                        target,
                        module.thunk_symbol(import.lookup + function as u32 * width)?,
                    )
                    .map_err(|error| {
                        module.error(import.lookup + function as u32 * width, error.reason)
                    })?;
                visitor(Fixup {
                    module: index,
                    rva: Rva(rva),
                    address: module.address(rva),
                    value: value.0,
                    kind: FixupKind::Import,
                    width: width as u8,
                })
                .map_err(VisitError::Callback)?;
            }
        }
        for descriptor in 0..module.delay_count()? {
            let import = module.delay_import(descriptor)?;
            let target = self
                .find_module(index, import.name, false)
                .map_err(|error| module.error(import.descriptor, error.reason))?;
            visitor(Fixup {
                module: index,
                rva: Rva(import.handle),
                address: module.address(import.handle),
                value: self.get(target).source.base.0,
                kind: FixupKind::ModuleHandle,
                width: width as u8,
            })
            .map_err(VisitError::Callback)?;
            for function in 0..import.count {
                let rva = import.iat + function as u32 * width;
                let name = import.names + function as u32 * width;
                let value = self
                    .resolve_export(target, module.thunk_symbol(name)?)
                    .map_err(|error| module.error(name, error.reason))?;
                visitor(Fixup {
                    module: index,
                    rva: Rva(rva),
                    address: module.address(rva),
                    value: value.0,
                    kind: FixupKind::Import,
                    width: width as u8,
                })
                .map_err(VisitError::Callback)?;
            }
        }
        Ok(())
    }
    pub(crate) fn validate_fixups(&self) -> Result<(), Error> {
        let limits = self.policy.limits();
        let mut imports = 0;
        let mut relocations = 0;
        for index in 0..self.count {
            let module = self.get(index);
            let width = module.pointer_bytes();
            // Slots no other fixup may touch: ordinary IATs with terminators,
            // delay module handles and the security cookie.
            let mut exclusive =
                [(0_u32, 0_u32); MAX_IMPORT_DESCRIPTORS + MAX_DELAY_DESCRIPTORS + 1];
            let mut exclusive_count = 0;
            // Delay IATs initially hold relocated helper-thunk pointers, so a
            // DIR64 record may precede the later import write on the same slot.
            let mut delay = [(0_u32, 0_u32); MAX_DELAY_DESCRIPTORS];
            let mut delay_count = 0;
            let mut claim =
                |rva: u32, bytes: u32, delay_iat: bool, error_rva: u32| -> Result<(), Error> {
                    let taken = exclusive[..exclusive_count]
                        .iter()
                        .chain(&delay[..delay_count])
                        .any(|&(start, length)| overlaps(rva, bytes, start, length));
                    if taken {
                        return Err(module.error(error_rva, Reason::OverlappingFixups));
                    }
                    if delay_iat {
                        delay[delay_count] = (rva, bytes);
                        delay_count += 1;
                    } else {
                        exclusive[exclusive_count] = (rva, bytes);
                        exclusive_count += 1;
                    }
                    Ok(())
                };
            for descriptor in 0..module.import_count()? {
                let import = module.import(descriptor)?;
                imports += import.count;
                if imports > limits.imports {
                    return Err(module.error(import.iat, Reason::ResourceLimit(Limit::Imports)));
                }
                claim(
                    import.iat,
                    (import.count + 1) as u32 * width,
                    false,
                    import.iat,
                )?;
            }
            for descriptor in 0..module.delay_count()? {
                let import = module.delay_import(descriptor)?;
                imports += import.count;
                if imports > limits.imports {
                    return Err(module.error(import.iat, Reason::ResourceLimit(Limit::Imports)));
                }
                claim(import.handle, width, false, import.handle)?;
                claim(import.iat, import.count as u32 * width, true, import.iat)?;
            }
            if let Some(cookie) = module.cookie_slot()? {
                claim(cookie, width, false, cookie)?;
            }
            let exclusive = &exclusive[..exclusive_count];
            module
                .walk_relocations(self.policy, |fixup| {
                    relocations += 1;
                    if relocations > limits.relocations {
                        return Err(
                            module.error(fixup.rva.0, Reason::ResourceLimit(Limit::Relocations))
                        );
                    }
                    for &(rva, bytes) in exclusive {
                        if overlaps(fixup.rva.0, width, rva, bytes) {
                            return Err(module.error(fixup.rva.0, Reason::OverlappingFixups));
                        }
                    }
                    Ok(())
                })
                .map_err(|error| match error {
                    VisitError::Plan(error) | VisitError::Callback(error) => error,
                })?;
            self.walk_imports(index, |_| Ok::<(), Infallible>(()))
                .map_err(infallible_plan_error)?;
        }
        Ok(())
    }
}

impl<'a> ModulePlan<'a> {
    pub(crate) fn import_count(&self) -> Result<usize, Error> {
        let count = self
            .image
            .import_count()
            .map_err(|error| self.error(0, Reason::Image(error)))?;
        if count > self.policy.limits().import_descriptors {
            return Err(self.error(0, Reason::ResourceLimit(Limit::Imports)));
        }
        Ok(count)
    }
    pub(crate) fn import(&self, index: usize) -> Result<Import<'a>, Error> {
        let (directory, _) = self.directory(1)?;
        let descriptor = self.read(directory + index as u32 * 20, 20)?;
        if dword(descriptor, 4) != 0 || dword(descriptor, 8) != 0 {
            return Err(self.error(directory, Reason::Unsupported(Feature::BoundImports)));
        }
        let iat = dword(descriptor, 16);
        let lookup = match dword(descriptor, 0) {
            0 => iat,
            other => other,
        };
        let width = self.pointer_bytes();
        if iat == 0 || !iat.is_multiple_of(width) || !lookup.is_multiple_of(width) {
            return Err(self.error(iat, Reason::InvalidImport));
        }
        let name = self
            .image
            .string(dword(descriptor, 12))
            .map_err(|_| self.error(directory, Reason::InvalidImport))?;
        for count in 0..=self.policy.limits().imports {
            let offset = count as u32 * width;
            let thunk_rva = lookup
                .checked_add(offset)
                .ok_or_else(|| self.error(lookup, Reason::InvalidImport))?;
            let thunk = self.pointer_at(thunk_rva)?;
            let iat_rva = iat
                .checked_add(offset)
                .ok_or_else(|| self.error(iat, Reason::InvalidImport))?;
            self.mapped_section(iat_rva, width)?;
            // Prebinding and synthesized IAT storage are outside P09: each
            // immutable initial slot must be the exact corresponding thunk.
            if self.pointer_at(iat_rva)? != thunk {
                return Err(self.error(iat_rva, Reason::InvalidImport));
            }
            if thunk == 0 {
                if count == 0 {
                    return Err(self.error(iat, Reason::InvalidImport));
                }
                return Ok(Import {
                    name,
                    lookup,
                    iat,
                    count,
                });
            }
        }
        Err(self.error(lookup, Reason::ResourceLimit(Limit::Imports)))
    }
    /// One import-name-table thunk: an ordinal or an RVA of a hint/name pair.
    pub(crate) fn thunk_symbol(&self, rva: u32) -> Result<Symbol<'a>, Error> {
        let thunk = self.pointer_at(rva)?;
        // IMAGE_ORDINAL_FLAG64 or IMAGE_ORDINAL_FLAG32.
        let (flag, reserved) = match self.machine() {
            Machine::X64 => (1 << 63, 0x7fff_ffff_ffff_0000),
            Machine::X86 => (1 << 31, 0x7fff_0000),
        };
        if thunk & flag != 0 {
            if thunk & reserved != 0 {
                return Err(self.error(rva, Reason::InvalidImport));
            }
            Ok(Symbol::Ordinal(thunk as u16))
        } else {
            let hint = u32::try_from(thunk)
                .ok()
                .filter(|rva| *rva != 0)
                .ok_or_else(|| self.error(rva, Reason::InvalidImport))?;
            self.read(hint, 2)?;
            let name = self
                .image
                .string(
                    hint.checked_add(2)
                        .ok_or_else(|| self.error(hint, Reason::InvalidImport))?,
                )
                .map_err(|_| self.error(hint, Reason::InvalidImport))?;
            Ok(Symbol::Name(name))
        }
    }
    /// A forwarder export's `module.symbol` text, or `None` for an address.
    pub(crate) fn forwarder(&self, rva: u32) -> Result<Option<(&'a str, Symbol<'a>)>, Error> {
        let (directory, bytes) = self.directory(0)?;
        if !(rva >= directory && rva < directory + bytes) {
            return Ok(None);
        }
        let text = self
            .image
            .string(rva)
            .map_err(|_| self.error(rva, Reason::InvalidExport))?;
        if u64::from(rva) + text.len() as u64 + 1 > u64::from(directory) + u64::from(bytes) {
            return Err(self.error(rva, Reason::InvalidExport));
        }
        let (name, symbol) = text
            .rsplit_once('.')
            .ok_or_else(|| self.error(rva, Reason::InvalidExport))?;
        let symbol = if let Some(ordinal) = symbol.strip_prefix('#') {
            if ordinal.is_empty() || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(self.error(rva, Reason::InvalidExport));
            }
            Symbol::Ordinal(
                ordinal
                    .parse()
                    .map_err(|_| self.error(rva, Reason::InvalidExport))?,
            )
        } else {
            if symbol.is_empty() {
                return Err(self.error(rva, Reason::InvalidExport));
            }
            Symbol::Name(symbol)
        };
        Ok(Some((name, symbol)))
    }
    fn export_rva(&self, symbol: Symbol<'_>) -> Result<u32, Error> {
        let (directory, _) = self.directory(0)?;
        if directory == 0 {
            return Err(self.error(0, Reason::MissingSymbol));
        }
        let table = self.read(directory, 40)?;
        let functions = dword(table, 20);
        let function = match symbol {
            Symbol::Ordinal(ordinal) => u32::from(ordinal)
                .checked_sub(dword(table, 16))
                .filter(|function| *function < functions),
            Symbol::Name(name) => {
                let mut found = None;
                for index in 0..dword(table, 24) {
                    let name_rva = dword(self.read(dword(table, 32) + index * 4, 4)?, 0);
                    if self
                        .image
                        .string(name_rva)
                        .map_err(|_| self.error(name_rva, Reason::InvalidExport))?
                        == name
                    {
                        found = Some(u32::from(word(
                            self.read(dword(table, 36) + index * 2, 2)?,
                            0,
                        )));
                        break;
                    }
                }
                found
            }
        }
        .ok_or_else(|| self.error(directory, Reason::MissingSymbol))?;
        let rva = dword(self.read(dword(table, 28) + function * 4, 4)?, 0);
        if rva == 0 {
            return Err(self.error(directory, Reason::MissingSymbol));
        }
        Ok(rva)
    }
}

fn overlaps(left: u32, left_bytes: u32, right: u32, right_bytes: u32) -> bool {
    u64::from(left) < u64::from(right) + u64::from(right_bytes)
        && u64::from(right) < u64::from(left) + u64::from(left_bytes)
}

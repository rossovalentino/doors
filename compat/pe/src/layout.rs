use core::convert::Infallible;

use door_image_format::Section;

use crate::{
    BundlePlan, Error, Feature, FileOffset, FileSpan, Limit, LoadPolicy, MAX_MODULE_BYTES,
    MAX_TLS_MODULES, Machine, ModulePlan, PAGE_BYTES, Permissions, Reason, Run, Rva,
    SUBSYSTEM_WINDOWS_CUI, SUBSYSTEM_WINDOWS_GUI, VisitError,
};

impl BundlePlan<'_> {
    pub(crate) fn validate_layout(&self) -> Result<(), Error> {
        let mut total_runs = 0;
        let mut tls_modules = 0;
        for index in 0..self.count {
            let module = self.get(index);
            if !valid_module_name(module.source.name)
                || (index == 0 && !has_extension(module.source.name, ".exe"))
                || (index != 0 && !has_extension(module.source.name, ".dll"))
            {
                return Err(module.error(0, Reason::InvalidModuleName));
            }
            module.validate_policy(self.policy)?;
            for previous in 0..index {
                let other = self.get(previous);
                if other.source.name.eq_ignore_ascii_case(module.source.name) {
                    return Err(module.error(0, Reason::DuplicateModule));
                }
                if module.source.base.0 < other.source.base.0 + u64::from(other.image_bytes())
                    && other.source.base.0 < module.source.base.0 + u64::from(module.image_bytes())
                {
                    return Err(module.error(0, Reason::AddressCollision));
                }
            }
            module
                .walk_runs(|_| {
                    total_runs += 1;
                    Ok::<(), Infallible>(())
                })
                .map_err(infallible_plan_error)?;
            if total_runs > self.policy.limits().runs {
                return Err(module.error(0, Reason::ResourceLimit(Limit::Runs)));
            }
            // x64 unwind data; x86 images register handlers through the SEH
            // chain and SafeSEH instead and carry no exception directory.
            match module.machine() {
                Machine::X64 => module.validate_unwind(self.policy.application())?,
                Machine::X86 => {
                    if module.directory(3)? != (0, 0) {
                        return Err(module.error(0, Reason::InvalidUnwind));
                    }
                }
            }
            if module.directory(9)? != (0, 0) {
                tls_modules += 1;
                if tls_modules > MAX_TLS_MODULES {
                    return Err(module.error(0, Reason::ResourceLimit(Limit::TlsModules)));
                }
            }
        }
        Ok(())
    }
}

impl ModulePlan<'_> {
    fn validate_policy(&self, policy: LoadPolicy) -> Result<(), Error> {
        if self.image.section_count() > policy.limits().sections {
            return Err(self.error(0, Reason::ResourceLimit(Limit::Sections)));
        }
        if self.image_bytes() == 0
            || self.image_bytes() > MAX_MODULE_BYTES
            || self.source.bytes.len() > MAX_MODULE_BYTES as usize
        {
            return Err(self.error(0, Reason::ResourceLimit(Limit::ImageBytes)));
        }
        let base = self.source.base.0;
        // An x86 image lies wholly below 2 GB: a 32-bit process's
        // addresses are widened with sign extension at the system call
        // boundary (WOW64 without /LARGEADDRESSAWARE gives it 2 GB too).
        let limit = match self.machine() {
            Machine::X64 => 0x0000_8000_0000_0000,
            Machine::X86 => 0x8000_0000,
        };
        if base < 0x1_0000
            || !base.is_multiple_of(0x1_0000)
            || base
                .checked_add(u64::from(self.image_bytes()))
                .is_none_or(|end| end > limit)
        {
            return Err(self.error(0, Reason::InvalidAddress));
        }
        // The subsystem selects the trusted process manifest (console or GUI);
        // it never grants a capability by itself. Native, POSIX, EFI and other
        // subsystems stay refused.
        let subsystem_allowed = self.image.subsystem == SUBSYSTEM_WINDOWS_CUI
            || (policy.application() && self.image.subsystem == SUBSYSTEM_WINDOWS_GUI);
        if !subsystem_allowed {
            return Err(self.error(0, Reason::Unsupported(Feature::Subsystem)));
        }
        let characteristics = self.image.characteristics;
        // An executable image (IMAGE_FILE_EXECUTABLE_IMAGE), a DLL exactly
        // where a DLL is loaded, and no system file (IMAGE_FILE_SYSTEM).
        // The other bits are hints Windows' loader ignores (stripped
        // symbols, BYTES_REVERSED_LO/HI, 32BIT_MACHINE, AGGRESSIVE_WS_TRIM,
        // LARGE_ADDRESS_AWARE, REMOVABLE/NET_RUN_FROM_SWAP, UP_SYSTEM_ONLY):
        // Delphi and C++Builder programs set BYTES_REVERSED, setup programs
        // linked with /SWAPRUN the RUN_FROM_SWAP bits.
        if characteristics & 2 == 0
            || (characteristics & 0x2000 != 0) != (self.index != 0)
            || characteristics & 0x1000 != 0
        {
            return Err(self.error(0, Reason::WrongImageKind));
        }
        if self.index != 0 && self.image.entry_rva != 0 && policy == LoadPolicy::Initial {
            return Err(self.error(
                self.image.entry_rva,
                Reason::Unsupported(Feature::DllInitialization),
            ));
        }
        let dll_flags = self
            .image
            .dll_characteristics()
            .map_err(|error| self.error(0, Reason::Image(error)))?;
        // Do not claim integrity, app-container or kernel-driver services.
        // ASLR hints, NX, NO_SEH and terminal-server awareness need no invented
        // callbacks in this fixed-address, exception-contained initial profile.
        // The application policy additionally admits manifest isolation and
        // binding hints (0x200, 0x800), which request no loader service, and
        // (P18) CFG-instrumented images (0x4000): as on a Windows that does
        // not enforce CFG, their own non-enforcing check and dispatch
        // routines stay in the load configuration's slots. Every MSVC
        // release of .NET, PowerShell and Chromium is built with /guard:cf.
        let mut allowed = 0x20 | 0x40 | 0x100 | 0x400 | 0x8000;
        if policy.application() {
            allowed |= 0x200 | 0x800 | 0x4000;
        }
        if dll_flags & !allowed != 0 {
            return Err(self.error(0, Reason::Unsupported(Feature::SecurityPolicy)));
        }
        for (directory, feature, application) in [
            (2, Feature::Resources, true),
            (7, Feature::Directory, false),
            (8, Feature::Directory, false),
            (10, Feature::LoadConfiguration, true),
            (11, Feature::BoundImports, false),
            (13, Feature::DelayImports, true),
            (14, Feature::ManagedImage, false),
            (15, Feature::Directory, false),
        ] {
            let (rva, bytes) = self.directory(directory)?;
            if (rva != 0 || bytes != 0) && !(application && policy.application()) {
                return Err(self.error(rva, Reason::Unsupported(feature)));
            }
        }
        if policy.application() {
            self.validate_resources()?;
            self.load_config()?;
            self.validate_delay_imports(policy)?;
        }
        if self.directory(9)? != (0, 0) {
            if policy == LoadPolicy::Initial {
                return Err(self.error(self.directory(9)?.0, Reason::Unsupported(Feature::Tls)));
            }
            self.tls()?;
        }
        // An ASLR image (DYNAMIC_BASE, relocations not stripped) with no base
        // relocation directory has no absolute addresses: x64 code is
        // RIP-relative, so Windows maps it anywhere without fixups. The earlier
        // policies keep requiring a directory to move an image.
        let position_independent = policy.application()
            && self.machine() == Machine::X64
            && characteristics & 1 == 0
            && dll_flags & 0x40 != 0
            && self.directory(5)? == (0, 0);
        if self.source.base.0 != self.image.image_base
            && (characteristics & 1 != 0 || self.directory(5)? == (0, 0))
            && !position_independent
        {
            return Err(self.error(0, Reason::RelocationRequired));
        }
        for index in 0..self.image.section_count() {
            permissions(self, self.section(index)?)?;
        }
        if self.index != 0 && self.image.entry_rva != 0 {
            let entry = self.image.entry_rva;
            let mut executable = false;
            self.walk_runs(|run| {
                if run.permissions == Permissions::ReadExecute
                    && entry >= run.rva.0
                    && run.file.is_some_and(|span| entry - run.rva.0 < span.bytes)
                {
                    executable = true;
                }
                Ok::<(), Infallible>(())
            })
            .map_err(infallible_plan_error)?;
            if !executable {
                return Err(self.error(entry, Reason::InvalidLayout));
            }
        }
        Ok(())
    }
    pub(crate) fn walk_runs<E>(
        &self,
        mut visitor: impl FnMut(Run) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        let headers = round_pages(self.image.headers_bytes())
            .ok_or_else(|| self.error(0, Reason::InvalidLayout))?;
        if headers > self.image_bytes() {
            return Err(self.error(0, Reason::InvalidLayout).into());
        }
        visitor(self.run(
            0,
            headers,
            Permissions::Read,
            Some(FileSpan {
                offset: FileOffset(0),
                bytes: self.image.headers_bytes(),
            }),
        ))
        .map_err(VisitError::Callback)?;
        let mut cursor = headers;
        for _ in 0..self.image.section_count() {
            let mut next: Option<Section> = None;
            for index in 0..self.image.section_count() {
                let section = self.section(index)?;
                if section.virtual_address >= cursor
                    && next.is_none_or(|prior| section.virtual_address < prior.virtual_address)
                {
                    next = Some(section);
                }
            }
            let section = next.ok_or_else(|| self.error(cursor, Reason::InvalidLayout))?;
            if section.virtual_address > cursor {
                visitor(self.run(
                    cursor,
                    section.virtual_address - cursor,
                    Permissions::None,
                    None,
                ))
                .map_err(VisitError::Callback)?;
            }
            let bytes = round_pages(section.mapped_bytes())
                .ok_or_else(|| self.error(section.virtual_address, Reason::InvalidLayout))?;
            let end = section
                .virtual_address
                .checked_add(bytes)
                .filter(|end| *end <= self.image_bytes())
                .ok_or_else(|| self.error(section.virtual_address, Reason::InvalidLayout))?;
            visitor(self.run(
                section.virtual_address,
                bytes,
                permissions(self, section)?,
                (section.raw_bytes != 0).then_some(FileSpan {
                    offset: FileOffset(section.raw_offset),
                    bytes: section.raw_bytes,
                }),
            ))
            .map_err(VisitError::Callback)?;
            cursor = end;
        }
        if cursor < self.image_bytes() {
            visitor(self.run(cursor, self.image_bytes() - cursor, Permissions::None, None))
                .map_err(VisitError::Callback)?;
        }
        Ok(())
    }
    fn run(&self, rva: u32, bytes: u32, permissions: Permissions, file: Option<FileSpan>) -> Run {
        Run {
            module: self.index,
            rva: Rva(rva),
            address: self.address(rva),
            bytes,
            permissions,
            file,
        }
    }
    pub(crate) fn section(&self, index: usize) -> Result<Section, Error> {
        self.image
            .section(index)
            .map_err(|error| self.error(0, Reason::Image(error)))
    }
    /// Extent must belong to one actual section, not its padding or a reserved gap.
    pub(crate) fn mapped_section(&self, rva: u32, bytes: u32) -> Result<Section, Error> {
        self.image
            .mapped_section(rva, bytes)
            .map_err(|_| self.error(rva, Reason::InvalidAddress))
    }
    pub(crate) fn executable(&self, rva: u32, bytes: u32) -> Result<(), Error> {
        let section = self.mapped_section(rva, bytes)?;
        if permissions(self, section)? != Permissions::ReadExecute {
            return Err(self.error(rva, Reason::InvalidExport));
        }
        self.read(rva, bytes as usize)?;
        Ok(())
    }
}

fn permissions(module: &ModulePlan<'_>, section: Section) -> Result<Permissions, Error> {
    match section.flags & 0xe000_0000 {
        0x4000_0000 => Ok(Permissions::Read),
        0xc000_0000 => Ok(Permissions::ReadWrite),
        0x6000_0000 => Ok(Permissions::ReadExecute),
        _ => Err(module.error(section.virtual_address, Reason::InvalidLayout)),
    }
}
fn round_pages(bytes: u32) -> Option<u32> {
    bytes
        .checked_add(PAGE_BYTES - 1)
        .map(|n| n & !(PAGE_BYTES - 1))
}
pub(crate) fn infallible_plan_error(error: VisitError<Infallible>) -> Error {
    match error {
        VisitError::Plan(error) => error,
        VisitError::Callback(never) => match never {},
    }
}
pub(crate) fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
pub(crate) fn has_extension(name: &str, extension: &str) -> bool {
    name.get(name.len().saturating_sub(extension.len())..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(extension))
}
pub(crate) fn section_permissions(
    module: &ModulePlan<'_>,
    section: Section,
) -> Result<Permissions, Error> {
    permissions(module, section)
}

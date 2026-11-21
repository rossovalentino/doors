//! Allocation-free plans for the Door Windows x64 personality.
//!
//! Parsing and planning do not grant authority or publish a runnable process.
//! The caller reserves every image extent, constructs all modules privately,
//! copies and patches them, applies final permissions, and rolls back on failure.
//! Immutable source bytes are borrowed for the complete construction transaction.
//!
//! Three explicit policies select the contract. `Initial` is the frozen P09
//! single-thread console profile, `DllCallbacks` the qualified P10 base profile,
//! and `Application` the P13 ordinary-application profile (console or GUI
//! subsystem, resources, load configuration and eagerly bound delay imports).
#![no_std]

pub mod closure;
mod delay;
pub mod image;
mod imports;
mod layout;
pub mod lifecycle;
mod load_config;
mod relocations;
pub mod resources;
pub mod tls;
mod unwind;
pub mod virtual_unwind;
pub mod x64_unwind;

pub use door_image_format::Error as ImageError;
pub use door_image_format::Machine;
use door_image_format::PeImage;
pub use load_config::LoadConfig;

/// Array capacities. Each policy selects its own, possibly lower, limit.
/// P18: a .NET program (PowerShell) loads most of the catalog; 32 leaves
/// room for the system DLLs it uses, the executable and private DLLs.
pub const MAX_MODULES: usize = 32;
pub const MAX_SECTIONS: usize = 16;
/// Includes inaccessible gaps, so reservation bookkeeping is bounded too.
pub const MAX_RUNS: usize = 768;
pub const MAX_IMPORTS: usize = 8192;
pub const MAX_IMPORT_DESCRIPTORS: usize = 32;
pub const MAX_DELAY_DESCRIPTORS: usize = 32;
pub const MAX_RELOCATIONS: usize = 65536;
/// Static compiler TLS templates share one native FS page per thread.
pub const MAX_TLS_MODULES: usize = 5;
pub const MAX_MODULE_BYTES: u32 = 16 * 1024 * 1024;
pub const PAGE_BYTES: u32 = 4096;
/// MSVC x64 `/GS` default; a loader must replace it before the image runs.
pub const DEFAULT_SECURITY_COOKIE: u64 = 0x0000_2b99_2ddf_a232;
/// MSVC x86 `/GS` default.
pub const DEFAULT_SECURITY_COOKIE_X86: u64 = 0xbb40_e64e;
pub const SUBSYSTEM_WINDOWS_GUI: u16 = 2;
pub const SUBSYSTEM_WINDOWS_CUI: u16 = 3;

/// API contracts (api-ms-win-*, ext-ms-win-*) and the DLL hosting each, as
/// Windows 11's API set schema resolves them: a name matches its contract
/// up to its last version number ("api-ms-win-core-memory-l1-1-6" is
/// "api-ms-win-core-memory-l1-1"), so every minor version of a contract
/// resolves and an unknown contract or level never does. kernelbase.dll's
/// contracts are hosted by kernel32.dll here; combase's by ole32.dll.
/// Import identity and exported behavior remain separate checks.
pub fn api_set_host(name: &str) -> Option<&'static str> {
    const KERNEL32: &str = "kernel32.dll";
    const NTDLL: &str = "ntdll.dll";
    const ADVAPI32: &str = "advapi32.dll";
    const UCRT: &str = "ucrtbase.dll";
    const OLE32: &str = "ole32.dll";
    const USER32: &str = "user32.dll";
    const SHELL32: &str = "shell32.dll";
    const GDI32: &str = "gdi32.dll";
    /// (contract without its last version number, host), sorted by name.
    const HOSTS: &[(&str, &str)] = &[
        ("api-ms-win-core-apiquery-l1-1", NTDLL),
        ("api-ms-win-core-apiquery-l2-1", KERNEL32),
        ("api-ms-win-core-appcompat-l1-1", KERNEL32),
        ("api-ms-win-core-atoms-l1-1", KERNEL32),
        ("api-ms-win-core-com-l1-1", OLE32),
        ("api-ms-win-core-com-l2-1", OLE32),
        ("api-ms-win-core-com-midlproxystub-l1-1", OLE32),
        ("api-ms-win-core-comm-l1-1", KERNEL32),
        ("api-ms-win-core-console-l1-1", KERNEL32),
        ("api-ms-win-core-console-l1-2", KERNEL32),
        ("api-ms-win-core-console-l2-1", KERNEL32),
        ("api-ms-win-core-console-l2-2", KERNEL32),
        ("api-ms-win-core-console-l3-1", KERNEL32),
        ("api-ms-win-core-console-l3-2", KERNEL32),
        ("api-ms-win-core-crt-l1-1", NTDLL),
        ("api-ms-win-core-crt-l2-1", KERNEL32),
        ("api-ms-win-core-datetime-l1-1", KERNEL32),
        ("api-ms-win-core-debug-l1-1", KERNEL32),
        ("api-ms-win-core-delayload-l1-1", KERNEL32),
        ("api-ms-win-core-enclave-l1-1", KERNEL32),
        ("api-ms-win-core-errorhandling-l1-1", KERNEL32),
        ("api-ms-win-core-fibers-l1-1", KERNEL32),
        ("api-ms-win-core-fibers-l2-1", KERNEL32),
        ("api-ms-win-core-file-l1-1", KERNEL32),
        ("api-ms-win-core-file-l1-2", KERNEL32),
        ("api-ms-win-core-file-l2-1", KERNEL32),
        ("api-ms-win-core-firmware-l1-1", KERNEL32),
        ("api-ms-win-core-handle-l1-1", KERNEL32),
        ("api-ms-win-core-heap-l1-1", KERNEL32),
        ("api-ms-win-core-heap-l2-1", KERNEL32),
        ("api-ms-win-core-heap-obsolete-l1-1", KERNEL32),
        ("api-ms-win-core-interlocked-l1-1", KERNEL32),
        ("api-ms-win-core-io-l1-1", KERNEL32),
        ("api-ms-win-core-job-l1-1", KERNEL32),
        ("api-ms-win-core-job-l2-1", KERNEL32),
        ("api-ms-win-core-kernel32-legacy-l1-1", KERNEL32),
        ("api-ms-win-core-kernel32-private-l1-1", KERNEL32),
        ("api-ms-win-core-largeinteger-l1-1", KERNEL32),
        ("api-ms-win-core-libraryloader-l1-1", KERNEL32),
        ("api-ms-win-core-libraryloader-l1-2", KERNEL32),
        ("api-ms-win-core-libraryloader-l2-1", KERNEL32),
        ("api-ms-win-core-localization-l1-1", KERNEL32),
        ("api-ms-win-core-localization-l1-2", KERNEL32),
        ("api-ms-win-core-localization-l2-1", KERNEL32),
        ("api-ms-win-core-localization-obsolete-l1-2", KERNEL32),
        ("api-ms-win-core-localization-obsolete-l1-3", KERNEL32),
        ("api-ms-win-core-localregistry-l1-1", ADVAPI32),
        ("api-ms-win-core-memory-l1-1", KERNEL32),
        ("api-ms-win-core-namedpipe-l1-1", KERNEL32),
        ("api-ms-win-core-namedpipe-l1-2", KERNEL32),
        ("api-ms-win-core-normalization-l1-1", KERNEL32),
        ("api-ms-win-core-path-l1-1", KERNEL32),
        ("api-ms-win-core-privateprofile-l1-1", KERNEL32),
        ("api-ms-win-core-processenvironment-l1-1", KERNEL32),
        ("api-ms-win-core-processenvironment-l1-2", KERNEL32),
        ("api-ms-win-core-processsnapshot-l1-1", KERNEL32),
        ("api-ms-win-core-processthreads-l1-1", KERNEL32),
        ("api-ms-win-core-processtopology-l1-1", KERNEL32),
        ("api-ms-win-core-processtopology-l1-2", KERNEL32),
        ("api-ms-win-core-processtopology-obsolete-l1-1", KERNEL32),
        ("api-ms-win-core-profile-l1-1", KERNEL32),
        ("api-ms-win-core-psapi-ansi-l1-1", KERNEL32),
        ("api-ms-win-core-psapi-l1-1", KERNEL32),
        ("api-ms-win-core-psapi-obsolete-l1-1", KERNEL32),
        ("api-ms-win-core-quirks-l1-1", KERNEL32),
        ("api-ms-win-core-realtime-l1-1", KERNEL32),
        ("api-ms-win-core-registry-l1-1", ADVAPI32),
        ("api-ms-win-core-registry-l2-1", ADVAPI32),
        ("api-ms-win-core-registry-l2-2", ADVAPI32),
        ("api-ms-win-core-rtlsupport-l1-1", NTDLL),
        ("api-ms-win-core-rtlsupport-l1-2", NTDLL),
        ("api-ms-win-core-shlwapi-legacy-l1-1", KERNEL32),
        ("api-ms-win-core-shlwapi-obsolete-l1-1", KERNEL32),
        ("api-ms-win-core-shlwapi-obsolete-l1-2", KERNEL32),
        ("api-ms-win-core-string-l1-1", KERNEL32),
        ("api-ms-win-core-string-l2-1", KERNEL32),
        ("api-ms-win-core-string-obsolete-l1-1", KERNEL32),
        ("api-ms-win-core-stringansi-l1-1", KERNEL32),
        ("api-ms-win-core-synch-ansi-l1-1", KERNEL32),
        ("api-ms-win-core-synch-l1-1", KERNEL32),
        ("api-ms-win-core-synch-l1-2", KERNEL32),
        ("api-ms-win-core-sysinfo-l1-1", KERNEL32),
        ("api-ms-win-core-sysinfo-l1-2", KERNEL32),
        ("api-ms-win-core-sysinfo-l2-1", KERNEL32),
        ("api-ms-win-core-systemtopology-l1-1", KERNEL32),
        ("api-ms-win-core-threadpool-l1-1", KERNEL32),
        ("api-ms-win-core-threadpool-l1-2", KERNEL32),
        ("api-ms-win-core-threadpool-legacy-l1-1", KERNEL32),
        ("api-ms-win-core-threadpool-private-l1-1", KERNEL32),
        ("api-ms-win-core-timezone-l1-1", KERNEL32),
        ("api-ms-win-core-toolhelp-l1-1", KERNEL32),
        ("api-ms-win-core-url-l1-1", KERNEL32),
        ("api-ms-win-core-util-l1-1", KERNEL32),
        ("api-ms-win-core-version-l1-1", KERNEL32),
        ("api-ms-win-core-versionansi-l1-1", KERNEL32),
        ("api-ms-win-core-windowserrorreporting-l1-1", KERNEL32),
        ("api-ms-win-core-winrt-error-l1-1", OLE32),
        ("api-ms-win-core-winrt-l1-1", OLE32),
        ("api-ms-win-core-winrt-string-l1-1", OLE32),
        ("api-ms-win-core-wow64-l1-1", KERNEL32),
        ("api-ms-win-core-xstate-l1-1", KERNEL32),
        ("api-ms-win-core-xstate-l2-1", KERNEL32),
        ("api-ms-win-crt-conio-l1-1", UCRT),
        ("api-ms-win-crt-convert-l1-1", UCRT),
        ("api-ms-win-crt-environment-l1-1", UCRT),
        ("api-ms-win-crt-filesystem-l1-1", UCRT),
        ("api-ms-win-crt-heap-l1-1", UCRT),
        ("api-ms-win-crt-locale-l1-1", UCRT),
        ("api-ms-win-crt-math-l1-1", UCRT),
        ("api-ms-win-crt-multibyte-l1-1", UCRT),
        ("api-ms-win-crt-private-l1-1", UCRT),
        ("api-ms-win-crt-process-l1-1", UCRT),
        ("api-ms-win-crt-runtime-l1-1", UCRT),
        ("api-ms-win-crt-stdio-l1-1", UCRT),
        ("api-ms-win-crt-string-l1-1", UCRT),
        ("api-ms-win-crt-time-l1-1", UCRT),
        ("api-ms-win-crt-utility-l1-1", UCRT),
        ("api-ms-win-downlevel-advapi32-l1-1", ADVAPI32),
        ("api-ms-win-downlevel-advapi32-l2-1", ADVAPI32),
        ("api-ms-win-downlevel-kernel32-l1-1", KERNEL32),
        ("api-ms-win-downlevel-kernel32-l2-1", KERNEL32),
        ("api-ms-win-downlevel-normaliz-l1-1", KERNEL32),
        ("api-ms-win-downlevel-ole32-l1-1", OLE32),
        ("api-ms-win-downlevel-shell32-l1-1", SHELL32),
        ("api-ms-win-downlevel-user32-l1-1", USER32),
        ("api-ms-win-downlevel-version-l1-1", KERNEL32),
        ("api-ms-win-eventing-classicprovider-l1-1", ADVAPI32),
        ("api-ms-win-eventing-consumer-l1-1", ADVAPI32),
        ("api-ms-win-eventing-controller-l1-1", ADVAPI32),
        ("api-ms-win-eventing-legacy-l1-1", ADVAPI32),
        ("api-ms-win-eventing-provider-l1-1", ADVAPI32),
        ("api-ms-win-gdi-dpiinfo-l1-1", GDI32),
        ("api-ms-win-power-base-l1-1", KERNEL32),
        ("api-ms-win-security-base-l1-1", ADVAPI32),
        ("api-ms-win-security-base-l1-2", ADVAPI32),
        ("api-ms-win-security-cryptoapi-l1-1", ADVAPI32),
        ("api-ms-win-security-lsalookup-l1-1", ADVAPI32),
        ("api-ms-win-security-lsalookup-l2-1", ADVAPI32),
        ("api-ms-win-security-provider-l1-1", ADVAPI32),
        ("api-ms-win-security-sddl-l1-1", ADVAPI32),
        ("api-ms-win-service-core-l1-1", ADVAPI32),
        ("api-ms-win-service-management-l1-1", ADVAPI32),
        ("api-ms-win-service-management-l2-1", ADVAPI32),
        ("api-ms-win-service-winsvc-l1-1", ADVAPI32),
        ("api-ms-win-service-winsvc-l1-2", ADVAPI32),
        ("api-ms-win-shcore-obsolete-l1-1", SHELL32),
        ("api-ms-win-shcore-scaling-l1-1", SHELL32),
        ("api-ms-win-shcore-stream-l1-1", SHELL32),
        ("api-ms-win-shell-shellcom-l1-1", SHELL32),
        ("api-ms-win-shell-shellfolders-l1-1", SHELL32),
        ("ext-ms-win-gdi-dc-l1-2", GDI32),
        ("ext-ms-win-gdi-draw-l1-1", GDI32),
        ("ext-ms-win-gdi-font-l1-1", GDI32),
        ("ext-ms-win-ntuser-message-l1-1", USER32),
        ("ext-ms-win-ntuser-misc-l1-1", USER32),
        ("ext-ms-win-ntuser-mouse-l1-1", USER32),
        ("ext-ms-win-ntuser-sysparams-ext-l1-1", USER32),
        ("ext-ms-win-ntuser-window-l1-1", USER32),
        ("ext-ms-win-ntuser-windowclass-l1-1", USER32),
        ("ext-ms-win-ntuser-windowstation-l1-1", USER32),
    ];
    let stem = name
        .len()
        .checked_sub(4)
        .filter(|&n| name.is_char_boundary(n) && name[n..].eq_ignore_ascii_case(".dll"))
        .map_or(name, |n| &name[..n]);
    // kernelbase.dll's functions live in kernel32.dll here, and shcore.dll's
    // in shell32.dll.
    if stem.eq_ignore_ascii_case("kernelbase") {
        return Some(KERNEL32);
    }
    if stem.eq_ignore_ascii_case("shcore") {
        return Some(SHELL32);
    }
    let (contract, version) = stem.rsplit_once('-')?;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    HOSTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(contract))
        .map(|(_, host)| *host)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FileOffset(pub u32);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Rva(pub u32);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ImageBase(pub u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct UserAddress(pub u64);

#[derive(Clone, Copy, Debug)]
pub struct ModuleSource<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub base: ImageBase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permissions {
    None,
    Read,
    ReadWrite,
    ReadExecute,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSpan {
    pub offset: FileOffset,
    pub bytes: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Run {
    pub module: usize,
    pub rva: Rva,
    pub address: UserAddress,
    pub bytes: u32,
    pub permissions: Permissions,
    /// Copy exactly these original bytes, then zero the rest of a committed run.
    /// A `None` permission run is reserved but never committed or accessible.
    pub file: Option<FileSpan>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixupKind {
    Dir64,
    /// IMAGE_REL_BASED_HIGHLOW: a 32-bit absolute address in an x86 image.
    HighLow,
    Import,
    /// Delay-import module handle slot, eagerly bound to the target base.
    ModuleHandle,
    /// `/GS` cookie slot. The value is the caller-supplied diversified cookie
    /// from `visit_fixups_with_cookie`; plain `visit_fixups` rewrites the
    /// image's compiled value (no entropy is invented by the planner).
    SecurityCookie,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fixup {
    pub module: usize,
    pub rva: Rva,
    pub address: UserAddress,
    pub value: u64,
    pub kind: FixupKind,
    /// Bytes written at `address`: the image's pointer width (8 for x64, 4
    /// for x86), whose value always fits.
    pub width: u8,
}
impl Fixup {
    /// The little-endian bytes to write: `width` of them.
    pub fn bytes(&self) -> ([u8; 8], usize) {
        (self.value.to_le_bytes(), usize::from(self.width))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Symbol<'a> {
    Name(&'a str),
    Ordinal(u16),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feature {
    Subsystem,
    DllInitialization,
    Tls,
    LoadConfiguration,
    DelayImports,
    BoundImports,
    ManagedImage,
    Resources,
    SecurityPolicy,
    Directory,
    UnwindHandler,
    UnwindVersion,
    /// Enforced Control Flow Guard (image requests CFG and is instrumented).
    ControlFlowGuard,
    /// Dynamic value relocations (retpoline/import control) need loader work.
    DynamicRelocations,
    /// CHPE/ARM64EC hybrid metadata.
    HybridImage,
    Enclave,
    CodeIntegrity,
    /// x86-only load-configuration tables (SafeSEH, lock prefixes, edit list).
    LegacyTables,
    /// VA-based (pre-VC7) delay descriptors.
    LegacyDelayImports,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Limit {
    Modules,
    Sections,
    Runs,
    ImageBytes,
    Imports,
    Relocations,
    Exports,
    Forwarders,
    UnwindEntries,
    Resources,
    TlsModules,
    DelayImports,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    Image(ImageError),
    InvalidModuleName,
    DuplicateModule,
    WrongImageKind,
    Unsupported(Feature),
    ResourceLimit(Limit),
    InvalidAddress,
    AddressCollision,
    MissingModule,
    MissingSymbol,
    DependencyCycle,
    ForwarderCycle,
    InvalidImport,
    InvalidExport,
    InvalidRelocation,
    RelocationRequired,
    InvalidUnwind,
    OverlappingFixups,
    InvalidLayout,
    DeferredDependency,
    InvalidResource,
    InvalidLoadConfiguration,
    InvalidDelayImport,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub module: usize,
    pub rva: Option<Rva>,
    pub reason: Reason,
}
impl Error {
    const fn new(module: usize, rva: Option<Rva>, reason: Reason) -> Self {
        Self {
            module,
            rva,
            reason,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisitError<E> {
    Plan(Error),
    Callback(E),
}
impl<E> From<Error> for VisitError<E> {
    fn from(error: Error) -> Self {
        Self::Plan(error)
    }
}

#[derive(Debug)]
pub struct ModulePlan<'a> {
    pub source: ModuleSource<'a>,
    pub index: usize,
    image: PeImage<'a>,
    policy: LoadPolicy,
}
impl<'a> ModulePlan<'a> {
    pub fn image_bytes(&self) -> u32 {
        self.image.image_bytes
    }
    /// x64, or x86 for a 32-bit image.
    pub fn machine(&self) -> Machine {
        self.image.machine
    }
    /// The image's pointer width: 8 bytes, or 4 for x86.
    pub fn pointer_bytes(&self) -> u32 {
        self.image.machine.pointer_bytes() as u32
    }
    /// A pointer-sized field of `bytes` at `offset` (checked extent).
    pub(crate) fn pointer_in(&self, bytes: &[u8], offset: usize) -> u64 {
        match self.image.machine {
            Machine::X64 => qword(bytes, offset),
            Machine::X86 => u64::from(dword(bytes, offset)),
        }
    }
    /// The pointer-sized value stored at `rva` (file-backed).
    pub(crate) fn pointer_at(&self, rva: u32) -> Result<u64, Error> {
        let width = self.pointer_bytes();
        Ok(self.pointer_in(self.read(rva, width as usize)?, 0))
    }
    pub fn preferred_base(&self) -> ImageBase {
        ImageBase(self.image.image_base)
    }
    /// Optional-header subsystem. Only 2 (GUI) and 3 (console) are admitted,
    /// and GUI only under `LoadPolicy::Application`.
    pub fn subsystem(&self) -> u16 {
        self.image.subsystem
    }
    pub fn entry(&self) -> Option<UserAddress> {
        (self.image.entry_rva != 0)
            .then(|| UserAddress(self.source.base.0 + u64::from(self.image.entry_rva)))
    }
    pub fn visit_runs<E>(
        &self,
        visitor: impl FnMut(Run) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        self.walk_runs(visitor)
    }
    fn error(&self, rva: u32, reason: Reason) -> Error {
        Error::new(self.index, Some(Rva(rva)), reason)
    }
    fn read(&self, rva: u32, length: usize) -> Result<&'a [u8], Error> {
        self.image
            .rva_bytes(rva, length)
            .map_err(|error| self.error(rva, Reason::Image(error)))
    }
    fn directory(&self, index: usize) -> Result<(u32, u32), Error> {
        self.image
            .directory(index)
            .map_err(|error| self.error(0, Reason::Image(error)))
    }
    fn address(&self, rva: u32) -> UserAddress {
        UserAddress(self.source.base.0 + u64::from(rva))
    }
    /// Translate an absolute pointer stored in the image file (relative to the
    /// preferred base) into an RVA inside this image, never a trusted address.
    fn preferred_rva(&self, value: u64, at: u32, reason: Reason) -> Result<u32, Error> {
        value
            .checked_sub(self.image.image_base)
            .and_then(|offset| u32::try_from(offset).ok())
            .filter(|offset| *offset < self.image_bytes())
            .ok_or_else(|| self.error(at, reason))
    }
}

/// Validated plans borrow immutable file bytes. No visitor is called during
/// construction, so malformed dependencies cannot cause partial external writes.
#[derive(Debug)]
pub struct BundlePlan<'a> {
    modules: [Option<ModulePlan<'a>>; MAX_MODULES],
    count: usize,
    order: [usize; MAX_MODULES],
    dependencies: [u64; MAX_MODULES],
    delay_dependencies: [u64; MAX_MODULES],
    policy: LoadPolicy,
}
/// Selecting a broader planner policy does not grant execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadPolicy {
    /// P09: one console executable and at most three DLLs without callbacks.
    Initial,
    /// P10: DLL callbacks, compiler TLS and API-set contracts.
    DllCallbacks,
    /// P13: ordinary console/GUI applications with resources, validated load
    /// configuration and eagerly bound delay imports.
    Application,
    /// P17: the same for 32-bit (x86, PE32) applications and DLLs; every
    /// module of the bundle is x86 and is placed below 4 GB.
    Application32,
}
/// Per-policy bounds. Every value is at most the matching array capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub modules: usize,
    pub sections: usize,
    pub runs: usize,
    pub imports: usize,
    pub import_descriptors: usize,
    pub relocations: usize,
}
impl LoadPolicy {
    pub const fn limits(self) -> Limits {
        match self {
            Self::Initial => Limits {
                modules: 4,
                sections: 8,
                runs: 24,
                imports: 1024,
                import_descriptors: 16,
                relocations: 4096,
            },
            Self::DllCallbacks => Limits {
                modules: 5,
                sections: 8,
                runs: 40,
                imports: 1024,
                import_descriptors: 16,
                relocations: 4096,
            },
            Self::Application | Self::Application32 => Limits {
                modules: MAX_MODULES,
                sections: MAX_SECTIONS,
                runs: MAX_RUNS,
                imports: MAX_IMPORTS,
                import_descriptors: MAX_IMPORT_DESCRIPTORS,
                relocations: MAX_RELOCATIONS,
            },
        }
    }
    /// Resources, load configuration, delay imports and the GUI subsystem.
    pub const fn application(self) -> bool {
        matches!(self, Self::Application | Self::Application32)
    }
    /// The machine every module of a bundle under this policy must have.
    pub const fn machine(self) -> Machine {
        match self {
            Self::Application32 => Machine::X86,
            _ => Machine::X64,
        }
    }
}
const _: () = {
    let limits = LoadPolicy::Application.limits();
    assert!(limits.modules <= MAX_MODULES && MAX_MODULES <= 64);
    assert!(limits.sections <= MAX_SECTIONS);
    assert!(limits.runs <= MAX_RUNS);
    assert!(limits.import_descriptors <= MAX_IMPORT_DESCRIPTORS);
};
impl<'a> BundlePlan<'a> {
    pub fn new(sources: &[ModuleSource<'a>]) -> Result<Self, Error> {
        Self::with_policy(sources, LoadPolicy::Initial)
    }
    pub fn with_policy(sources: &[ModuleSource<'a>], policy: LoadPolicy) -> Result<Self, Error> {
        if sources.is_empty() || sources.len() > policy.limits().modules {
            return Err(Error::new(0, None, Reason::ResourceLimit(Limit::Modules)));
        }
        let mut plan = Self {
            modules: [const { None }; MAX_MODULES],
            count: sources.len(),
            order: [0; MAX_MODULES],
            dependencies: [0; MAX_MODULES],
            delay_dependencies: [0; MAX_MODULES],
            policy,
        };
        for (index, source) in sources.iter().copied().enumerate() {
            plan.modules[index] = Some(ModulePlan {
                source,
                index,
                image: parse(source.bytes, policy)
                    .map_err(|error| Error::new(index, None, Reason::Image(error)))?,
                policy,
            });
        }
        plan.validate_layout()?;
        plan.validate_dependencies()?;
        plan.validate_fixups()?;
        Ok(plan)
    }
    pub fn module_count(&self) -> usize {
        self.count
    }
    pub fn policy(&self) -> LoadPolicy {
        self.policy
    }
    /// Subsystem of the executable (module zero).
    pub fn subsystem(&self) -> u16 {
        self.get(0).subsystem()
    }
    /// A deferred module cannot satisfy an import, delay import or forwarder in
    /// a startup image. Validate this before publishing otherwise valid IATs.
    pub fn validate_deferred_module(&self, index: usize) -> Result<(), Error> {
        if index == 0 || index >= self.count {
            return Err(Error::new(index, None, Reason::MissingModule));
        }
        for from in 0..self.count {
            if from != index
                && (self.dependencies[from] | self.delay_dependencies[from]) & (1u64 << index) != 0
            {
                return Err(Error::new(from, None, Reason::DeferredDependency));
            }
        }
        Ok(())
    }
    pub fn module(&self, index: usize) -> Option<&ModulePlan<'a>> {
        self.modules.get(index).and_then(Option::as_ref)
    }
    pub fn entry(&self) -> UserAddress {
        // Successful construction guarantees a nonzero executable entry.
        self.get(0).address(self.get(0).image.entry_rva)
    }
    pub fn dependency_order(&self) -> &[usize] {
        &self.order[..self.count]
    }
    /// Stable static TLS slot (0..MAX_TLS_MODULES) of a module with a TLS
    /// directory, in module-index order. The bound is checked at construction.
    pub fn tls_slot(&self, index: usize) -> Option<usize> {
        let has = |module: usize| {
            self.module(module)
                .is_some_and(|plan| plan.directory(9).is_ok_and(|dir| dir != (0, 0)))
        };
        has(index).then(|| (0..index).filter(|module| has(*module)).count())
    }
    pub fn resolve_export(&self, module: usize, symbol: Symbol<'_>) -> Result<UserAddress, Error> {
        if module >= self.count {
            return Err(Error::new(module, None, Reason::MissingModule));
        }
        self.resolve(module, symbol, &mut [(usize::MAX, 0); 16], 0)
    }
    /// Resolve a function the trusted startup path will actually call. Ordinary
    /// PE imports may legitimately designate data and use `resolve_export`.
    pub fn resolve_executable_export(
        &self,
        module: usize,
        symbol: Symbol<'_>,
    ) -> Result<UserAddress, Error> {
        let address = self.resolve_export(module, symbol)?;
        for index in 0..self.count {
            let target = self.get(index);
            if let Some(rva) = address
                .0
                .checked_sub(target.source.base.0)
                .filter(|rva| *rva < u64::from(target.image_bytes()))
            {
                target.executable(rva as u32, 1)?;
                return Ok(address);
            }
        }
        Err(Error::new(module, None, Reason::InvalidExport))
    }
    pub fn visit_runs<E>(
        &self,
        mut visitor: impl FnMut(Run) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        for &index in self.dependency_order() {
            self.get(index).walk_runs(&mut visitor)?;
        }
        Ok(())
    }
    /// Applies relocation records first, then IAT and delay-import records,
    /// then security cookies. Every target and resolved value was checked
    /// before this plan was returned. The callback's original error survives;
    /// the caller must roll back any partial mapping. Cookie slots receive the
    /// image's compiled value; callers that run code use the cookie variant.
    pub fn visit_fixups<E>(
        &self,
        visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        self.walk_fixups(None, visitor)
    }
    /// As `visit_fixups`, but every `/GS` cookie slot that still holds zero or
    /// the compiler default receives a value diversified from `seed` (per
    /// module, high 16 bits clear, never zero or the default), as the Windows
    /// loader does. `seed` must come from the caller's own entropy/diversity.
    pub fn visit_fixups_with_cookie<E>(
        &self,
        seed: u64,
        visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        self.walk_fixups(Some(seed), visitor)
    }
    fn walk_fixups<E>(
        &self,
        seed: Option<u64>,
        mut visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        for &index in self.dependency_order() {
            self.get(index)
                .walk_relocations(self.policy, &mut visitor)?;
        }
        for &index in self.dependency_order() {
            self.walk_imports(index, &mut visitor)?;
        }
        for &index in self.dependency_order() {
            self.get(index).walk_cookie(seed, &mut visitor)?;
        }
        Ok(())
    }
    fn get(&self, index: usize) -> &ModulePlan<'a> {
        self.modules[index]
            .as_ref()
            .expect("validated module index")
    }
}

/// Initial and P10 policies keep refusing a delay-import directory inside the
/// shared parser; only the P13 policy parses and binds it explicitly.
pub(crate) fn parse(bytes: &[u8], policy: LoadPolicy) -> Result<PeImage<'_>, ImageError> {
    match policy {
        LoadPolicy::Application32 => {
            let image = PeImage::parse_any_with_delay_imports(bytes)?;
            if image.machine != Machine::X86 {
                return Err(ImageError::Unsupported);
            }
            Ok(image)
        }
        LoadPolicy::Application => PeImage::parse_with_delay_imports(bytes),
        _ => PeImage::parse(bytes),
    }
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("checked extent"),
    )
}
fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked extent"),
    )
}
fn qword(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked extent"),
    )
}

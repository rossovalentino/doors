//! Trusted P09 image catalog and Windows startup policy, separate from PE syntax.
mod diagnostics;
pub(super) mod exception;
use super::*;
use door_pe_loader::{BundlePlan, ImageBase, LoadPolicy, ModuleSource, Symbol};
use door_windows_abi::{self as win, Bootstrap, ModuleInit, modules};
use sha2::{Digest, Sha256};

static IMAGES: Storage<[&'static [u8]; 3]> = Storage(UnsafeCell::new(None));
static BASE_IMAGES: Storage<[&'static [u8]; 5]> = Storage(UnsafeCell::new(None));
/// Authenticated system DLL catalog of ordinary applications, indexed like
/// `modules::PROVIDERS`; a provider absent from initfs cannot be imported.
static PROVIDERS: Storage<[Option<&'static [u8]>; modules::PROVIDERS.len()]> =
    Storage(UnsafeCell::new(None));
/// The 32-bit builds (P17, `windows32/`), indexed like `modules::PROVIDERS32`.
static PROVIDERS32: Storage<[Option<&'static [u8]>; modules::PROVIDERS32.len()]> =
    Storage(UnsafeCell::new(None));
static VERIFIED: AtomicBool = AtomicBool::new(false);
static APPLICATION_VERIFIED: AtomicBool = AtomicBool::new(false);
pub(super) struct Image {
    pub plan: BundlePlan<'static>,
    pub entry: u64,
    manifest: u64,
    exception_entry: u64,
}
/// The P13 application path: an ordinary console (10) or GUI (15) image
/// captured by PROCESS_EXECUTE, planned over its own import closure.
pub(super) fn application_manifest(manifest: u64) -> bool {
    matches!(
        manifest,
        door_abi::platform::manifest::WINDOWS_APPLICATION
            | door_abi::platform::manifest::WINDOWS_GUI
    )
}
/// Manifest of a captured PE image by optional-header subsystem: GUI (2)
/// selects WINDOWS_GUI, console (3) WINDOWS_APPLICATION; others are refused.
pub(super) fn subsystem_manifest(executable: &[u8]) -> Result<u64, Status> {
    // x64, or x86 when the 32-bit DLLs are present (P17).
    let image = door_image_format::PeImage::parse_any_with_delay_imports(executable)
        .map_err(|_| Status::InvalidArgument)?;
    if image.machine == door_image_format::Machine::X86 && !x86_available() {
        return Err(Status::Unsupported);
    }
    match image.subsystem {
        door_pe_loader::SUBSYSTEM_WINDOWS_GUI => Ok(door_abi::platform::manifest::WINDOWS_GUI),
        door_pe_loader::SUBSYSTEM_WINDOWS_CUI => {
            Ok(door_abi::platform::manifest::WINDOWS_APPLICATION)
        }
        _ => Err(Status::Unsupported),
    }
}
pub(super) fn initialize(archive: &door_initfs::Archive<'static>) -> Result<(), Status> {
    let images = [
        "windows/windows-basic.exe",
        "windows/ntdll.dll",
        "windows/kernel32.dll",
    ]
    .map(|path| archive.file(path).ok_or(Status::NotFound));
    let [exe, ntdll, kernel32] = images;
    let images = [exe?, ntdll?, kernel32?];
    unsafe {
        if (*IMAGES.0.get()).is_some() {
            return Err(Status::AlreadyExists);
        }
        *IMAGES.0.get() = Some(images);
    }
    diagnostics::verify(&sources(images))?;
    if let (Some(exe), Some(dll), Some(crt)) = (
        archive.file("windows/windows-base.exe"),
        archive.file("windows/lifecycle.dll"),
        archive.file("windows/msvcrt.dll"),
    ) {
        unsafe {
            *BASE_IMAGES.0.get() = Some([exe, images[1], images[2], dll, crt]);
        }
    }
    Ok(())
}
fn failure(error: door_pe_loader::Error) -> Status {
    log(format_args!(
        "DOOR_P09 LOAD_FAIL module={} rva={:#x} reason={:?}\n",
        error.module,
        error.rva.map_or(0, |rva| rva.0),
        error.reason
    ));
    Status::InvalidArgument
}
pub(super) fn image(manifest: u64) -> Result<Image, Status> {
    if manifest == door_abi::platform::manifest::WINDOWS_BASE {
        let images = unsafe { (*BASE_IMAGES.0.get()).as_ref().ok_or(Status::NotFound)? };
        let names = [
            "windows-base.exe",
            "ntdll.dll",
            "kernel32.dll",
            "lifecycle.dll",
            "msvcrt.dll",
        ];
        let bases = [
            0x1_4200_0000,
            0x1_8000_0000,
            0x1_8200_0000,
            0x1_8400_0000,
            0x1_8600_0000,
        ];
        let inputs = core::array::from_fn::<_, 5, _>(|index| ModuleSource {
            name: names[index],
            bytes: images[index],
            base: ImageBase(bases[index]),
        });
        let plan = BundlePlan::with_policy(&inputs, door_pe_loader::LoadPolicy::DllCallbacks)
            .map_err(failure)?;
        plan.validate_deferred_module(3).map_err(failure)?;
        let entry = plan
            .resolve_executable_export(1, Symbol::Name(win::BOOTSTRAP_EXPORT))
            .map_err(failure)?
            .0;
        let exception_entry = plan
            .resolve_executable_export(1, Symbol::Name(win::exception::DISPATCH_EXPORT))
            .map_err(failure)?
            .0;
        return Ok(Image {
            plan,
            entry,
            manifest,
            exception_entry,
        });
    }
    let images = unsafe { (*IMAGES.0.get()).as_ref().ok_or(Status::NotFound)? };
    let plan = BundlePlan::new(&sources(*images)).map_err(failure)?;
    let entry = plan
        .resolve_executable_export(1, Symbol::Name(win::BOOTSTRAP_EXPORT))
        .map_err(failure)?
        .0;
    Ok(Image {
        plan,
        entry,
        manifest,
        exception_entry: 0,
    })
}
fn sources(images: [&[u8]; 3]) -> [ModuleSource<'_>; 3] {
    [
        ModuleSource {
            name: "windows-basic.exe",
            bytes: images[0],
            base: ImageBase(0x1_4200_0000),
        },
        ModuleSource {
            name: "ntdll.dll",
            bytes: images[1],
            base: ImageBase(0x1_8000_0000),
        },
        ModuleSource {
            name: "kernel32.dll",
            bytes: images[2],
            base: ImageBase(0x1_8200_0000),
        },
    ]
}
impl Image {
    pub fn exception_entry(&self) -> u64 {
        self.exception_entry
    }
    /// DOORWIN2 (application path: every module's extent and validated
    /// resource directory) or the frozen P09/P10 DOORWIN1 record. Returns the
    /// encoded length; module records follow dependency initialization order.
    fn encode_bootstrap(
        &self,
        output: &mut [u8; win::bootstrap::BOOTSTRAP_BYTES],
    ) -> Result<usize, Status> {
        let module_init = |index: usize| {
            let module = self.plan.module(index).expect("validated dependency");
            let (resource_rva, resource_bytes) = module
                .resource_directory()
                .map_or((0, 0), |(rva, bytes)| (rva.0, bytes));
            win::bootstrap::ModuleInit {
                base: module.source.base.0,
                entry: module.entry().map_or(0, |entry| entry.0),
                image_bytes: module.image_bytes().next_multiple_of(4096),
                resource_rva,
                resource_bytes,
                flags: 0,
            }
        };
        let order = self
            .plan
            .dependency_order()
            .iter()
            .copied()
            .filter(|index| *index != 0);
        if application_manifest(self.manifest) {
            let mut modules = [win::bootstrap::ModuleInit::default(); win::bootstrap::MAX_MODULES];
            let mut count = 0;
            for index in order {
                *modules.get_mut(count).ok_or(Status::ResourceExhausted)? = module_init(index);
                count += 1;
            }
            let executable = module_init(0);
            win::bootstrap::Bootstrap {
                native_startup: 0x7100_0000,
                executable_entry: self.plan.entry().0,
                executable_base: executable.base,
                teb: win::TEB_ADDRESS,
                peb: win::PEB_ADDRESS,
                parameters: win::PARAMETERS_ADDRESS,
                subsystem: self.plan.subsystem(),
                executable: win::bootstrap::ModuleInit {
                    entry: self.plan.entry().0,
                    ..executable
                },
                module_count: count as u32,
                modules,
            }
            .encode(output)
            .map_err(|_| Status::InvalidArgument)?;
            return Ok(win::bootstrap::BOOTSTRAP_BYTES);
        }
        let mut modules = [ModuleInit::default(); win::MAX_MODULES];
        let mut count = 0;
        for index in order {
            *modules.get_mut(count).ok_or(Status::ResourceExhausted)? = ModuleInit {
                base: self.plan.module(index).expect("validated").source.base.0,
                // The P10 lifecycle DLL is attached only by its dynamic load.
                entry: if self.manifest == door_abi::platform::manifest::WINDOWS_BASE && index == 3
                {
                    0
                } else {
                    module_init(index).entry
                },
            };
            count += 1;
        }
        Bootstrap {
            native_startup: 0x7100_0000,
            executable_entry: self.plan.entry().0,
            executable_base: self.plan.module(0).expect("validated").source.base.0,
            teb: win::TEB_ADDRESS,
            peb: win::PEB_ADDRESS,
            parameters: win::PARAMETERS_ADDRESS,
            module_count: count as u32,
            modules,
        }
        .encode(&mut output[..win::BOOTSTRAP_BYTES])
        .map_err(|_| Status::InvalidArgument)?;
        Ok(win::BOOTSTRAP_BYTES)
    }
    /// Bind the trusted, fully installed image inventory to its process ID.
    /// Emitted outside the metadata guard, before making the context runnable.
    pub fn report(&self, pid: u64) {
        for index in 0..self.plan.module_count() {
            let module = self.plan.module(index).expect("validated module");
            let digest = Sha256::digest(module.source.bytes);
            let mut hex = [0; 64];
            for (out, value) in hex.chunks_exact_mut(2).zip(digest) {
                out[0] = b"0123456789abcdef"[(value >> 4) as usize];
                out[1] = b"0123456789abcdef"[(value & 15) as usize];
            }
            log(format_args!(
                "DOOR_P09 MODULE pid={pid} index={index} base={:#x} preferred={:#x} bytes={} sha256={}\n",
                module.source.base.0,
                module.preferred_base().0,
                module.source.bytes.len(),
                core::str::from_utf8(&hex).expect("hexadecimal digest")
            ));
        }
        log(format_args!(
            "DOOR_P09 EXECUTION pid={pid} manifest={} modules={} entry={:#x} executable={:#x} fs=0x71002000 gs={:#x}\n",
            self.manifest,
            self.plan.module_count(),
            self.entry,
            self.plan.entry().0,
            win::TEB_ADDRESS
        ));
    }
    pub fn executable(&self) -> &'static [u8] {
        self.plan
            .module(0)
            .expect("validated PE executable")
            .source
            .bytes
    }
    /// Runs only while the process is unpublished and the VM guard is held.
    pub unsafe fn create(
        &self,
        vm: &mut Vm,
        slot: usize,
        startup: &door_abi::platform::StartupV2,
        native: &[u8],
    ) -> Result<vm::UserLayout, Status> {
        self.with_inputs(startup, native, |inputs| unsafe {
            self.create_with(vm, slot, inputs)
        })
    }
    /// P13 application-path rollback self-test, once per boot before the
    /// first ordinary application (7-16 modules, static TLS) is created, as
    /// the P09 path verifies its first launch. Every construction checkpoint
    /// (each run, fixup and protection pass, common mapping and TLS install)
    /// is failed once through production construction and teardown, and
    /// each attempt must leave the frame, process and section counts, the
    /// static TLS slots and every page of the slot's arena as they were.
    /// Each attempt holds the VM guard alone, so a large bundle never stalls
    /// other address-space users for the whole sweep. Must run while the
    /// slot is reserved and unpublished, without the VM guard.
    pub fn verify_application_rollback(
        &self,
        slot: usize,
        startup: &door_abi::platform::StartupV2,
        native: &[u8],
    ) -> Result<(), Status> {
        if !application_manifest(self.manifest) || APPLICATION_VERIFIED.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut runs = 0usize;
        self.plan
            .visit_runs(|_| {
                runs += 1;
                Ok::<_, Status>(())
            })
            .map_err(|_| Status::InvalidArgument)?;
        let result = self.with_inputs(startup, native, |inputs| {
            for limit in 0..=vm::MAX_APPLICATION_CHECKPOINTS {
                let outcome = with_vm(|v| unsafe {
                    v.verify_windows_application_attempt(slot, &self.plan, inputs, limit)
                })?;
                if let Some(tls) = outcome {
                    return Ok((limit, tls));
                }
            }
            Err(Status::IoFailure)
        });
        match result {
            Ok((checkpoints, tls)) => {
                APPLICATION_VERIFIED.store(true, Ordering::Release);
                log(format_args!(
                    "DOOR_P13 APP_ROLLBACK PASS checkpoints={checkpoints} modules={} runs={runs} tls={tls} leaks=0\n",
                    self.plan.module_count()
                ));
                Ok(())
            }
            Err(error) => {
                log(format_args!(
                    "DOOR_P13 APP_ROLLBACK FAIL modules={} runs={runs} status={error:?}\n",
                    self.plan.module_count()
                ));
                Err(error)
            }
        }
    }
    fn with_inputs<R>(
        &self,
        startup: &door_abi::platform::StartupV2,
        native: &[u8],
        f: impl FnOnce(&vm::WindowsStartup<'_>) -> Result<R, Status>,
    ) -> Result<R, Status> {
        let mut bootstrap_bytes = [0; win::bootstrap::BOOTSTRAP_BYTES];
        let bootstrap_length = self.encode_bootstrap(&mut bootstrap_bytes)?;
        let bootstrap_bytes = &bootstrap_bytes[..bootstrap_length];
        let mut state = [0; win::STATE_BYTES];
        let mut image_path = [0u16; 128];
        let mut command_line = [0u16; 256];
        let (path_units, command_units) = if application_manifest(self.manifest) {
            let launched = &startup.arguments[..startup.argument_bytes as usize];
            let native_first = launched
                .split(|b| *b == 0)
                .next()
                .ok_or(Status::InvalidArgument)?;
            // A program on C: is named by its Win32 path in ImagePathName
            // and argv[0], as Windows does (`C:\Program Files\...`, not the
            // native `/compat/Program Files/...`); the .NET host and others
            // resolve their own folder from it.
            let mut rewritten = [0u8; door_abi::platform::ARGUMENT_BYTES + 8];
            let args = match win::windows_image_path(native_first, &mut rewritten) {
                Some(n) => {
                    let tail = &launched[native_first.len()..];
                    let total = n + tail.len();
                    rewritten
                        .get_mut(n..total)
                        .ok_or(Status::InvalidArgument)?
                        .copy_from_slice(tail);
                    &rewritten[..total]
                }
                None => launched,
            };
            let first = args
                .split(|b| *b == 0)
                .next()
                .ok_or(Status::InvalidArgument)?;
            let first = core::str::from_utf8(first).map_err(|_| Status::InvalidArgument)?;
            let mut path_units = 0;
            for unit in first.encode_utf16() {
                if path_units == image_path.len() {
                    return Err(Status::InvalidArgument);
                }
                image_path[path_units] = unit;
                path_units += 1;
            }
            // cmd.exe/Explorer form: only arguments that need it are quoted.
            let command_units =
                win::command_line(args, &mut command_line).map_err(|_| Status::InvalidArgument)?;
            (path_units, command_units)
        } else {
            let path: &[u8] = if self.manifest == door_abi::platform::manifest::WINDOWS_BASE {
                b"C:\\system\\windows-base.exe"
            } else {
                b"C:\\system\\windows-basic.exe"
            };
            for (out, &byte) in image_path.iter_mut().zip(path) {
                *out = u16::from(byte);
            }
            command_line[..path.len()].copy_from_slice(&image_path[..path.len()]);
            let mut units = path.len();
            if startup.mode == door_abi::platform::mode::WINDOWS_REOPEN {
                for &byte in b" --reopen" {
                    command_line[units] = u16::from(byte);
                    units += 1;
                }
            }
            (path.len(), units)
        };
        let session =
            u32::try_from(startup.identity.session).map_err(|_| Status::ResourceExhausted)?;
        if self.plan.policy() == LoadPolicy::Application32 {
            // A 32-bit program: its TEB32 and PEB32 (P17).
            win::initial_state32(
                &mut state,
                Vm::service_stack_top(),
                0x7000_1000,
                session,
                &image_path[..path_units],
                &command_line[..command_units],
                self.plan
                    .module(0)
                    .ok_or(Status::InvalidArgument)?
                    .source
                    .base
                    .0,
                Vm::main_tls() + win::x86::PAGE_TLS_ARRAY as u64,
            )
            .map_err(|_| Status::InvalidArgument)?;
        } else {
            win::initial_state(
                &mut state,
                Vm::service_stack_top(),
                0x7000_1000,
                session,
                &image_path[..path_units],
                &command_line[..command_units],
            )
            .map_err(|_| Status::InvalidArgument)?;
        }
        if application_manifest(self.manifest) {
            win::initial_environment(
                &mut state,
                &startup.environment[..startup.environment_bytes as usize],
            )
            .map_err(|_| Status::InvalidArgument)?;
            win::startup::prepare(
                &mut state,
                self.manifest == door_abi::platform::manifest::WINDOWS_GUI,
            )
            .map_err(|_| Status::InvalidArgument)?;
        }
        let inputs = vm::WindowsStartup {
            native,
            bootstrap: bootstrap_bytes,
            state: &state,
            entry: self.entry,
            pid: startup.base.pid,
        };
        f(&inputs)
    }
    unsafe fn create_with(
        &self,
        vm: &mut Vm,
        slot: usize,
        inputs: &vm::WindowsStartup<'_>,
    ) -> Result<vm::UserLayout, Status> {
        if !application_manifest(self.manifest) && !VERIFIED.load(Ordering::Acquire) {
            let checkpoints = unsafe { vm.verify_windows_rollback(slot, &self.plan, inputs) }?;
            VERIFIED.store(true, Ordering::Release);
            log(format_args!(
                "DOOR_P09 VM PASS rollback={} permissions=1 guards=1\n",
                checkpoints
            ));
        }
        let layout = unsafe { vm.create_windows_process(slot, &self.plan, inputs) }?;
        if self.manifest == door_abi::platform::manifest::WINDOWS_BASE {
            match unsafe { vm.verify_windows_module_rollback(slot, &self.plan) } {
                Ok(checkpoints) => log(format_args!(
                    "DOOR_P10 MODULE_ROLLBACK PASS checkpoints={checkpoints} mappings=0 tls=0\n"
                )),
                Err(error) => {
                    unsafe { vm.destroy_process(slot) }?;
                    return Err(error);
                }
            }
        }
        Ok(layout)
    }
}

/// Ordinary execution admits caller-supplied EXE bytes and the authenticated
/// system DLLs of the fixed catalog. ntdll and kernel32 are required; the
/// others (msvcrt, user32, gdi32, comdlg32, comctl32, ws2_32, iphlpapi, dnsapi, winhttp, wininet, winmm,
/// advapi32, dsound, ole32, shell32) are optional, and an
/// image importing an absent provider fails to load (no search path).
pub(super) fn initialize_providers(archive: &door_initfs::Archive<'static>) -> Result<(), Status> {
    let mut catalog = [None; modules::PROVIDERS.len()];
    for (slot, provider) in catalog.iter_mut().zip(&modules::PROVIDERS) {
        if provider.system {
            *slot = archive.file(provider.path);
        }
    }
    if catalog[modules::NTDLL].is_none() || catalog[modules::KERNEL32].is_none() {
        return Err(Status::NotFound);
    }
    unsafe {
        *PROVIDERS.0.get() = Some(catalog);
    }
    // The 32-bit builds are optional: without ntdll and kernel32 no x86
    // program starts.
    let mut catalog32 = [None; modules::PROVIDERS32.len()];
    for (slot, provider) in catalog32.iter_mut().zip(&modules::PROVIDERS32) {
        if provider.system {
            *slot = archive.file(provider.path);
        }
    }
    if catalog32[modules::NTDLL].is_some() && catalog32[modules::KERNEL32].is_some() {
        unsafe {
            *PROVIDERS32.0.get() = Some(catalog32);
        }
    }
    Ok(())
}
/// Whether 32-bit programs can run (their ntdll and kernel32 are present).
pub(super) fn x86_available() -> bool {
    unsafe { (*PROVIDERS32.0.get()).is_some() }
}
/// Plan an ordinary application over exactly its import, delay-import and
/// forwarder closure (P13 `Application` policy, up to 32 modules). ntdll is
/// always module 1 (it hosts the bootstrap). The executable's subsystem must
/// match the launch manifest: GUI (2) for WINDOWS_GUI, console (3) otherwise.
pub(super) fn application(executable: &'static [u8], manifest: u64) -> Result<Image, Status> {
    application_with(executable, manifest, &[], &[])
}
/// A load-order entry naming private module `index` (see `application_with`).
pub(super) const PRIVATE_ROOT: u8 = 0x80;
/// The application's plan after LoadLibrary brought in the system DLLs
/// `roots` (catalog indices, in load order): the startup closure keeps its
/// module indices, and each root adds the modules its closure needs that
/// were not there yet, in catalog order, after everything loaded before it.
/// A root `PRIVATE_ROOT | k` is the private DLL `privates[k]` (not a
/// system DLL: an application's own), placed at its private slot after the
/// system DLLs its imports need.
pub(super) fn application_with(
    executable: &'static [u8],
    manifest: u64,
    roots: &[u8],
    privates: &[&'static [u8]],
) -> Result<Image, Status> {
    if !application_manifest(manifest) || subsystem_manifest(executable)? != manifest {
        return Err(Status::InvalidArgument);
    }
    // P17: an x86 executable is planned over the 32-bit catalog, below 4 GB.
    let image = door_image_format::PeImage::parse_any_with_delay_imports(executable)
        .map_err(|_| Status::InvalidArgument)?;
    let x86 = image.machine == door_image_format::Machine::X86;
    let (catalog, table, policy): (_, &[modules::Provider; modules::PROVIDER_COUNT], _) = if x86 {
        (
            unsafe { (*PROVIDERS32.0.get()).as_ref().ok_or(Status::NotFound)? },
            &modules::PROVIDERS32,
            LoadPolicy::Application32,
        )
    } else {
        (
            unsafe { (*PROVIDERS.0.get()).as_ref().ok_or(Status::NotFound)? },
            &modules::PROVIDERS,
            LoadPolicy::Application,
        )
    };
    let private_base = |k: usize| {
        if x86 {
            modules::private32_base(k)
        } else {
            modules::private_base(k)
        }
    };
    let executable_base = if x86 {
        modules::executable32_base(image.image_base, u64::from(image.image_bytes))
    } else {
        modules::EXECUTABLE_BASE
    };
    let empty = ModuleSource {
        name: "",
        bytes: &[],
        base: ImageBase(0),
    };
    let mut providers = [empty; modules::PROVIDERS.len()];
    let mut count = 0;
    let mut ntdll = None;
    for (index, provider) in table.iter().enumerate() {
        let Some(bytes) = catalog[index] else {
            continue;
        };
        if index == modules::NTDLL {
            ntdll = Some(count);
        }
        providers[count] = ModuleSource {
            name: provider.name,
            bytes,
            base: ImageBase(provider.base),
        };
        count += 1;
    }
    let ntdll = ntdll.ok_or(Status::NotFound)?;
    let exe = ModuleSource {
        name: "application.exe",
        bytes: executable,
        base: ImageBase(executable_base),
    };
    let closure =
        door_pe_loader::closure::import_closure(exe, &providers[..count], &[ntdll], policy)
            .map_err(failure)?;
    let mut order = [0usize; door_pe_loader::MAX_MODULES];
    let mut ordered = 0;
    for &index in closure.providers() {
        order[ordered] = index;
        ordered += 1;
    }
    // Order entries below `count` are provider positions; PRIVATE + k is
    // private module k.
    const PRIVATE: usize = 1 << 16;
    let private = |k: usize| -> Result<ModuleSource<'static>, Status> {
        Ok(ModuleSource {
            name: modules::PRIVATE_NAMES
                .get(k)
                .copied()
                .ok_or(Status::InvalidArgument)?,
            bytes: privates.get(k).copied().ok_or(Status::InvalidArgument)?,
            base: ImageBase(private_base(k)),
        })
    };
    let mut required = [ntdll; door_pe_loader::MAX_MODULES];
    let mut system = 0;
    for &root in roots {
        let grown = if root & PRIVATE_ROOT != 0 {
            // A private DLL: the system DLLs its own imports need, then it.
            let k = usize::from(root & !PRIVATE_ROOT);
            door_pe_loader::closure::import_closure(
                private(k)?,
                &providers[..count],
                &[ntdll],
                policy,
            )
            .map_err(failure)?
        } else {
            // Catalog index to provider position.
            let position = (0..count)
                .find(|&p| providers[p].base.0 == table[usize::from(root)].base)
                .ok_or(Status::NotFound)?;
            system += 1;
            *required.get_mut(system).ok_or(Status::ResourceExhausted)? = position;
            door_pe_loader::closure::import_closure(
                exe,
                &providers[..count],
                &required[..=system],
                policy,
            )
            .map_err(failure)?
        };
        for position in 0..count {
            if grown.providers().contains(&position) && !order[..ordered].contains(&position) {
                *order.get_mut(ordered).ok_or(Status::ResourceExhausted)? = position;
                ordered += 1;
            }
        }
        if root & PRIVATE_ROOT != 0 {
            *order.get_mut(ordered).ok_or(Status::ResourceExhausted)? =
                PRIVATE + usize::from(root & !PRIVATE_ROOT);
            ordered += 1;
        }
    }
    let mut inputs = [exe; door_pe_loader::MAX_MODULES];
    if 1 + ordered > inputs.len() {
        return Err(Status::ResourceExhausted);
    }
    for (input, &index) in inputs[1..].iter_mut().zip(&order[..ordered]) {
        *input = if index >= PRIVATE {
            private(index - PRIVATE)?
        } else {
            providers[index]
        };
    }
    let plan = BundlePlan::with_policy(&inputs[..1 + ordered], policy).map_err(failure)?;
    let entry = plan
        .resolve_executable_export(1, Symbol::Name(win::BOOTSTRAP_EXPORT))
        .map_err(failure)?
        .0;
    let exception_entry = plan
        .resolve_executable_export(1, Symbol::Name(win::exception::DISPATCH_EXPORT))
        .map_err(failure)?
        .0;
    Ok(Image {
        plan,
        entry,
        exception_entry,
        manifest,
    })
}

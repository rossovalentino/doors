#![no_std]
#![no_main]
extern crate alloc;
mod removal;
mod replacement;
mod replacement_wire;
mod storage_reconnect;
mod window;
use alloc::vec::Vec;
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};
use door_abi::{
    Status,
    platform::{HEAP_BYTES, StartupV2, inherited, mode},
};
use door_service_client::{
    Console, Disposition, Error as ClientError, READ, RegistryRoot, RegistryValue, RegistryView,
    SHARE_ALL, Setting, Storage, WRITE,
};
use door_service_protocol::Code;

#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
static EVENT: AtomicU64 = AtomicU64::new(0);
static BUFFER: AtomicU64 = AtomicU64::new(0);
static WRITING: AtomicU64 = AtomicU64::new(0);
static DESTRUCTORS: AtomicU64 = AtomicU64::new(0);
static STAGE: AtomicU64 = AtomicU64::new(0);
static RELEASE_WORKER: AtomicU64 = AtomicU64::new(0);
static REVOCATION_WORKER_READY: AtomicU64 = AtomicU64::new(0);
static REVOCATION_CONDITION: AtomicU64 = AtomicU64::new(0);
door_rt::entry_v2!(main);

#[derive(Debug)]
enum Error {
    Sys(Status),
    Service(ClientError),
    Check(&'static str),
}
impl From<Status> for Error {
    fn from(v: Status) -> Self {
        Self::Sys(v)
    }
}
impl From<ClientError> for Error {
    fn from(v: ClientError) -> Self {
        Self::Service(v)
    }
}
type Result<T> = core::result::Result<T, Error>;
struct Text {
    bytes: [u8; 192],
    used: usize,
}
impl Text {
    fn new() -> Self {
        Self {
            bytes: [0; 192],
            used: 0,
        }
    }
    fn text(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.used]).unwrap_or("invalid")
    }
}
impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.used + s.len() > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.used..self.used + s.len()].copy_from_slice(s.as_bytes());
        self.used += s.len();
        Ok(())
    }
}
fn log(args: fmt::Arguments<'_>) {
    let mut text = Text::new();
    let _ = text.write_fmt(args);
    let _ = door_sys::write(text.text().as_bytes());
}
fn fail(error: &Error) {
    match error {
        Error::Sys(s) => log(format_args!(
            "DOOR_P08 APP ERROR stage={} syscall={s:?}\n",
            STAGE.load(Ordering::Acquire)
        )),
        Error::Service(s) => log(format_args!(
            "DOOR_P08 APP ERROR stage={} service={s:?}\n",
            STAGE.load(Ordering::Acquire)
        )),
        Error::Check(s) => log(format_args!(
            "DOOR_P08 APP ERROR stage={} check={s}\n",
            STAGE.load(Ordering::Acquire)
        )),
    }
}
fn check(value: bool, name: &'static str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error::Check(name))
    }
}
fn path(uid: u32) -> Result<&'static str> {
    match uid {
        1001 => Ok("/users/1001/workload.bin"),
        1002 => Ok("/users/1002/workload.bin"),
        _ => Err(Error::Check("provisioned identity")),
    }
}
fn pattern(index: usize, uid: u32) -> u8 {
    ((index * 29 + if uid == 1001 { 17 } else { 53 }) % 251) as u8
}
fn checked_rename_roundtrip(
    storage: &mut Storage,
    file: &door_service_client::File,
    uid: u32,
) -> Result<()> {
    let home = if uid == 1001 {
        "/users/1001"
    } else {
        "/users/1002"
    };
    let parent = storage.metadata(home)?.object;
    let original = storage.info(file)?;
    let name: Vec<_> = "workload.bin".encode_utf16().collect();
    check(
        matches!(
            storage.rename_child_checked(parent, original.object, original.version + 1, &name),
            Err(ClientError::Service(Code::Changed))
        ),
        "rename stale entry",
    )?;
    let blocker = storage.open(path(uid)?, Disposition::OpenExisting, READ, READ | WRITE)?;
    check(
        matches!(
            storage.rename_child_checked(parent, original.object, original.version, &name),
            Err(ClientError::Service(Code::SharingViolation))
        ),
        "rename delete sharing",
    )?;
    storage.close(blocker)?;
    check(
        matches!(
            storage.rename_child_checked(parent, original.object, original.version, &[47]),
            Err(ClientError::Service(Code::InvalidRequest))
        ),
        "rename invalid component",
    )?;
    let foreign = if uid == 1001 {
        "/users/1002"
    } else {
        "/users/1001"
    };
    // Knowing the other user's numeric object identity must not bypass traversal.
    check(
        matches!(
            storage.metadata(foreign),
            Err(ClientError::Service(Code::Denied))
        ),
        "rename foreign ancestry remains denied",
    )?;
    // This fixture provisions the two home objects as17/18. A known numeric
    // parent is still a hint, and must not bypass its current traversal ACL.
    let foreign_parent = if uid == 1001 { 18 } else { 17 };
    check(
        matches!(
            storage.rename_child_checked(foreign_parent, original.object, original.version, &name),
            Err(ClientError::Service(Code::Denied))
        ),
        "rename guessed foreign parent denied",
    )?;
    let raw = [0xd800, b'.' as u16, b'b' as u16, b'i' as u16, b'n' as u16];
    storage.rename_child_checked(parent, original.object, original.version, &raw)?;
    let changed = storage.info(file)?;
    check(
        changed.object == original.object
            && changed.version == original.version + 1
            && changed.bytes == original.bytes,
        "rename stable open file",
    )?;
    check(
        matches!(
            storage.metadata(path(uid)?),
            Err(ClientError::Service(Code::NotFound))
        ),
        "rename old path removed",
    )?;
    let mut after = 0;
    let mut found = false;
    while let Some(entry) = storage.list(home, after)? {
        after = entry.cookie;
        if entry.info.object == original.object {
            found = entry.name[..entry.name_len] == raw;
            break;
        }
    }
    check(found, "rename raw UTF16 listing")?;
    check(
        matches!(
            storage.rename_child_checked(parent, original.object, original.version, &name),
            Err(ClientError::Service(Code::Changed))
        ),
        "rename stale original after commit",
    )?;
    storage.rename_child_checked(parent, original.object, changed.version, &name)?;
    check(
        storage.metadata(path(uid)?)?.object == original.object,
        "rename restored original identity",
    )?;
    fill_half(storage, file, uid, 0, 4096, false)?;
    log(format_args!(
        "DOOR_P12 SDK RENAME PASS uid={uid} durable_commits=2 raw_utf16=1 identity=1 sharing=1 stale=1\n"
    ));
    Ok(())
}
fn measure_rpc_latency(storage: &mut Storage, uid: u32) -> Result<()> {
    // Explicit private SDK exercise only; no mutation and no console/log I/O
    // inside the measured loop. Compare every reply with the same actual file.
    let expected = storage.metadata(path(uid)?)?;
    let started = door_sys::time()?;
    for _ in 0..128 {
        check(
            storage.metadata(path(uid)?)? == expected,
            "RPC measured metadata identity",
        )?;
    }
    let elapsed = door_sys::time()?
        .checked_sub(started)
        .ok_or(Error::Check("RPC clock order"))?;
    log(format_args!(
        "DOOR_P12 SDK RPC PASS calls=128 ticks={elapsed} identity=1\n"
    ));
    Ok(())
}
fn fill_half(
    storage: &mut Storage,
    file: &door_service_client::File,
    uid: u32,
    start: usize,
    end: usize,
    write: bool,
) -> Result<()> {
    let mut bytes = [0; 192];
    let mut offset = start;
    while offset < end {
        let count = (end - offset).min(bytes.len());
        if write {
            for (i, b) in bytes[..count].iter_mut().enumerate() {
                *b = pattern(offset + i, uid);
            }
            check(
                storage.write(file, offset as u64, &bytes[..count])? == count,
                "exact write",
            )?;
        } else {
            check(
                storage.read(file, offset as u64, &mut bytes[..count])? == count,
                "exact read",
            )?;
            check(
                bytes[..count]
                    .iter()
                    .enumerate()
                    .all(|(i, b)| *b == pattern(offset + i, uid)),
                "persisted bytes",
            )?;
        }
        offset += count;
    }
    Ok(())
}
unsafe extern "C" fn destructor(value: u64) {
    if door_rt::thread::get_local() == Ok(value) {
        DESTRUCTORS.fetch_add(1, Ordering::Release);
    }
}
fn worker(registry: u64) -> Result<()> {
    let start_deadline = door_sys::time()? + 100;
    while RELEASE_WORKER.load(Ordering::Acquire) == 0 {
        let now = door_sys::time()?;
        check(now < start_deadline, "worker start deadline")?;
        door_sys::sleep(now + 1)?;
    }
    door_rt::thread::set_local(0x2222)?;
    unsafe {
        door_rt::thread::register_destructor(destructor, 0x2222)?;
    }
    let mut allocated = Vec::new();
    allocated
        .try_reserve_exact(32768)
        .map_err(|_| Error::Check("worker allocation"))?;
    allocated.resize(32768, 0x5au8);
    check(
        allocated.iter().all(|b| *b == 0x5a),
        "worker allocation bytes",
    )?;
    let pointer = BUFFER.load(Ordering::Acquire) as *const u8;
    check(!pointer.is_null(), "shared heap publication")?;
    // Parent retains this allocation until joining this thread; release/acquire
    // publishes initialized bytes. No writer changes them while the worker reads.
    check(
        unsafe { core::slice::from_raw_parts(pointer, 65536) }
            .iter()
            .enumerate()
            .all(|(i, b)| *b == (i % 251) as u8),
        "shared heap contents",
    )?;
    let identity = door_sys::identity(0)?;
    let mut storage = Storage::connect(registry)?;
    let file = storage.open(
        path(identity.uid)?,
        Disposition::OpenExisting,
        READ | WRITE,
        SHARE_ALL,
    )?;
    fill_half(
        &mut storage,
        &file,
        identity.uid,
        2048,
        4096,
        WRITING.load(Ordering::Acquire) != 0,
    )?;
    storage.close(file)?;
    let deadline = door_sys::time()?
        .checked_add(3)
        .ok_or(Error::Check("clock overflow"))?;
    door_sys::sleep(deadline)?;
    check(door_sys::time()? >= deadline, "timer deadline")?;
    check(door_rt::thread::get_local()? == 0x2222, "thread TLS")?;
    drop(allocated);
    door_sys::signal(EVENT.load(Ordering::Acquire), 1)?;
    Ok(())
}
unsafe extern "C" fn worker_entry(registry: u64) -> ! {
    let result = worker(registry);
    let code = match result {
        Ok(()) => 42,
        Err(error) => {
            fail(&error);
            -250
        }
    };
    door_rt::thread::finish(code)
}

fn registry_roundtrip(storage: &mut Storage, uid: u32, writing: bool) -> Result<()> {
    const KEY: &[u16] = &[68, 111, 111, 114];
    const VALUE: &[u16] = &[69, 112, 111, 99, 104];
    check(
        matches!(
            storage.registry_open(
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                KEY,
                WRITE,
                true
            ),
            Err(ClientError::Service(Code::Denied))
        ),
        "machine registry write denied",
    )?;
    check(
        matches!(
            storage.registry_open(
                RegistryRoot::CurrentUser,
                RegistryView::Bits32,
                KEY,
                READ,
                false
            ),
            Err(ClientError::Service(Code::NotFound))
        ),
        "registry views remain separate",
    )?;
    let key = storage.registry_open(
        RegistryRoot::CurrentUser,
        RegistryView::Bits64,
        KEY,
        if writing { READ | WRITE } else { READ },
        writing,
    )?;
    if writing {
        storage.registry_set(&key, VALUE, RegistryValue::dword(uid))?;
    }
    check(
        storage.registry_query(&key, VALUE)? == RegistryValue::dword(uid),
        "typed per-user registry value",
    )?;
    storage.registry_close(key)?;
    let key = storage.registry_open(
        RegistryRoot::CurrentUser,
        RegistryView::Bits64,
        KEY,
        READ,
        false,
    )?;
    check(
        matches!(
            storage.registry_set(&key, VALUE, RegistryValue::dword(0)),
            Err(ClientError::Service(Code::Denied))
        ),
        "read-only registry grant denies mutation",
    )?;
    check(
        storage.registry_query(&key, &[101, 112, 111, 99, 104])? == RegistryValue::dword(uid),
        "registry value survives denial and case-insensitive query",
    )?;
    storage.registry_close(key)?;
    log(format_args!(
        "DOOR_P08 REGISTRY PASS uid={uid} view=64 epoch={uid} denied=1\n"
    ));
    Ok(())
}

fn run(startup: &StartupV2) -> Result<()> {
    STAGE.store(1, Ordering::Release);
    let own = door_sys::identity(0)?;
    check(own == startup.identity, "kernel identity matches startup")?;
    check(
        own.capabilities == 0 && own.session != 0 && own.epoch != 0,
        "ordinary session authority",
    )?;
    check(
        startup.argument_bytes != 0 && startup.environment_bytes != 0,
        "arguments and environment",
    )?;
    check(
        door_sys::inspect(startup.cwd)?[0] == door_abi::HandleKind::Directory as u64,
        "cwd capability",
    )?;
    let entropy = door_sys::entropy_status()?;
    check(
        door_abi::entropy::well_formed(entropy) && entropy & door_abi::entropy::READY != 0,
        "kernel entropy ready",
    )?;
    // A signed-in session's apps are served from their own DRBG (TLS keys,
    // DNS transaction IDs; the kernel's ENTROPY_READ policy since P14/P15):
    // the bytes arrive, and they are not the buffer's old contents.
    let mut served = [0x5a; 16];
    check(
        door_sys::entropy_read(&mut served).is_ok() && served != [0x5a; 16],
        "entropy bytes served to session apps",
    )?;
    check(
        matches!(
            door_sys::block_info(0),
            Err(Status::BadHandle | Status::AccessDenied)
        ),
        "raw disk denied",
    )?;
    check(
        matches!(
            door_sys::process_launch(0, 2, 0, 0, 1),
            Err(Status::BadHandle | Status::AccessDenied)
        ),
        "service launch denied",
    )?;
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    check(registry != 0, "registry capability")?;
    if startup.mode == mode::SAVE && own.uid == 1001 {
        let mut console = Console::connect(registry)?;
        // An ordinary console client cannot manufacture trusted serial lines.
        // The host's strict process/marker parser rejects these if framing breaks.
        console.write("\r\nDOOR_P08 PROCESS manifest=5 pid=999 instance=999 uid=1001 session=1 epoch=1 exit=71\n")?;
        console.write("\nDOOR_USER pid=999 DOOR_P08 APP PASS uid=1001 bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1\r\n")?;
        console.write("partial console text without newline")?;
    }
    HEAP.initialize()?;
    check(
        unsafe {
            door_sys::vm_allocate(
                door_abi::platform::HEAP_BASE + HEAP_BYTES,
                4096,
                door_abi::rights::READ | door_abi::rights::WRITE,
            )
        } == Err(Status::ResourceExhausted),
        "manifest heap budget",
    )?;
    door_rt::thread::set_local(0x1111)?;
    let mut parent = Vec::new();
    parent
        .try_reserve_exact(65536)
        .map_err(|_| Error::Check("parent allocation"))?;
    parent.resize(65536, 0u8);
    for (i, b) in parent.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let mut pressure: Vec<u8> = Vec::new();
    check(
        pressure.try_reserve_exact(HEAP_BYTES as usize * 2).is_err(),
        "contained OOM",
    )?;
    BUFFER.store(parent.as_ptr() as u64, Ordering::Release);
    let writing = startup.mode == mode::SAVE;
    WRITING.store(u64::from(writing), Ordering::Release);
    STAGE.store(2, Ordering::Release);
    let mut storage = Storage::connect(registry)?;
    let other = if own.uid == 1001 { 1002 } else { 1001 };
    check(
        matches!(
            storage.open(path(other)?, Disposition::OpenExisting, READ, SHARE_ALL),
            Err(ClientError::Service(Code::Denied))
        ),
        "cross-user file denial",
    )?;
    let file = storage.open(
        path(own.uid)?,
        if writing {
            Disposition::CreateNew
        } else {
            Disposition::OpenExisting
        },
        READ | WRITE,
        SHARE_ALL,
    )?;
    let mut conflict = Storage::connect(registry)?;
    check(
        matches!(
            conflict.open(path(own.uid)?, Disposition::OpenExisting, WRITE, 0),
            Err(ClientError::Service(Code::SharingViolation))
        ),
        "symmetric sharing denial",
    )?;
    drop(conflict);
    STAGE.store(3, Ordering::Release);
    let event = door_sys::wait_create(1, 0, 0)?;
    EVENT.store(event, Ordering::Release);
    let thread = unsafe { door_rt::thread::spawn(worker_entry, registry) }?;
    check(
        unsafe { door_rt::thread::spawn(worker_entry, registry) } == Err(Status::ResourceExhausted),
        "manifest thread budget",
    )?;
    RELEASE_WORKER.store(1, Ordering::Release);
    fill_half(&mut storage, &file, own.uid, 0, 2048, writing)?;
    let deadline = door_sys::time()? + 1000;
    door_sys::wait(event, deadline)?;
    check(
        door_rt::thread::join(thread, deadline)? == 42,
        "thread join result",
    )?;
    check(
        DESTRUCTORS.load(Ordering::Acquire) == 1,
        "worker TLS destructor",
    )?;
    check(
        door_rt::thread::get_local()? == 0x1111,
        "independent main TLS",
    )?;
    door_sys::close(thread)?;
    door_sys::close(event)?;
    let mutex = door_sys::wait_create(2, 0, 0)?;
    let condition = door_sys::wait_create(3, 0, 0)?;
    door_sys::wait(mutex, door_sys::time()? + 50)?;
    check(
        door_sys::condition_wait(condition, mutex, door_sys::time()? + 2) == Err(Status::Timeout),
        "condition timeout reacquires mutex",
    )?;
    door_sys::signal(mutex, 1)?;
    door_sys::close(condition)?;
    door_sys::close(mutex)?;
    let timeout = door_sys::wait_create(1, 0, 1)?;
    check(
        door_sys::wait(timeout, door_sys::time()? + 2) == Err(Status::Timeout),
        "event timeout",
    )?;
    door_sys::close(timeout)?;
    STAGE.store(4, Ordering::Release);
    let (theme, scale) = if own.uid == 1001 { (1, 125) } else { (2, 150) };
    registry_roundtrip(&mut storage, own.uid, writing)?;
    if writing {
        storage.set_setting(Setting::Theme, theme)?;
        storage.set_setting(Setting::ScalePercent, scale)?;
        storage.flush()?;
    }
    check(
        storage.get_setting(Setting::Theme)? == theme
            && storage.get_setting(Setting::ScalePercent)? == scale,
        "typed settings",
    )?;
    storage.close(file)?;
    let file = storage.open(path(own.uid)?, Disposition::OpenExisting, READ, SHARE_ALL)?;
    fill_half(&mut storage, &file, own.uid, 0, 4096, false)?;
    storage.close(file)?;
    drop(storage);
    drop(parent);
    drop(pressure);
    BUFFER.store(0, Ordering::Release);
    check(
        HEAP.stats().used == 0 && HEAP.stats().allocations == 0,
        "heap fully reclaimed",
    )?;
    log(format_args!(
        "DOOR_P08 APP PASS uid={} bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1\n",
        own.uid
    ));
    if startup.mode == mode::DEMO {
        interactive(registry, own.uid)?;
    }
    unsafe {
        HEAP.shutdown()?;
    }
    Ok(())
}
fn interactive(registry: u64, uid: u32) -> Result<()> {
    let mut console = Console::connect(registry)?;
    console.write("Door OS native console\nServices, private files and protected threads are running.\nType help for commands.\n")?;
    log(format_args!("DOOR_P08 CONSOLE READY\n"));
    let mut input = [0u8; 64];
    let mut count = 0;
    console.write("door> ")?;
    loop {
        let key = match console.read() {
            Ok(key) => key,
            Err(ClientError::Service(Code::WouldBlock))
            | Err(ClientError::Transport(Status::WouldBlock)) => {
                door_sys::sleep(door_sys::time()? + 1)?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        match key {
            b'\n' => {
                console.write("\n")?;
                let command = core::str::from_utf8(&input[..count])
                    .map_err(|_| Error::Check("console UTF8"))?;
                match command {
                    "help" => {
                        console.write("help  status  file  theme  threads  exit\n")?;
                    }
                    "status" => {
                        let id = door_sys::identity(0)?;
                        let mut text = Text::new();
                        let _ = writeln!(
                            text,
                            "UID {} / session {} / heap {} bytes in use",
                            id.uid,
                            id.session,
                            HEAP.stats().used
                        );
                        console.write(text.text())?;
                    }
                    "file" => {
                        let mut storage = Storage::connect(registry)?;
                        let file =
                            storage.open(path(uid)?, Disposition::OpenExisting, READ, SHARE_ALL)?;
                        fill_half(&mut storage, &file, uid, 0, 4096, false)?;
                        storage.close(file)?;
                        console.write("workload.bin: all 4096 saved bytes verified\n")?;
                    }
                    "theme" => {
                        let mut storage = Storage::connect(registry)?;
                        let next = (storage.get_setting(Setting::Theme)? + 1) % 3;
                        storage.set_setting(Setting::Theme, next)?;
                        storage.flush()?;
                        check(storage.get_setting(Setting::Theme)? == next, "saved theme")?;
                        let mut text = Text::new();
                        let _ = writeln!(text, "Saved native theme {next}");
                        console.write(text.text())?;
                    }
                    "rename" => {
                        // Explicit SDK exercise only. The normal save, reopen,
                        // restart and console workflows retain their disk contract.
                        let mut storage = Storage::connect(registry)?;
                        let file = storage.open(
                            path(uid)?,
                            Disposition::OpenExisting,
                            READ | WRITE,
                            SHARE_ALL,
                        )?;
                        checked_rename_roundtrip(&mut storage, &file, uid)?;
                        storage.close(file)?;
                        measure_rpc_latency(&mut storage, uid)?;
                        console
                            .write("Checked rename restored workload.bin and all 4096 bytes\n")?;
                    }
                    "replacedoc" => {
                        replacement::exercise(registry, uid)?;
                        console.write("Checked replacement: private staging, conflict rejection, maximum and empty saves passed\n")?;
                    }
                    "reconnect" => {
                        storage_reconnect::exercise(registry, uid)?;
                        console.write("Storage reconnect:64 retired channels, stale tokens denied, saved bytes unchanged\n")?;
                    }
                    "removeprep" => {
                        removal::prepare(registry, uid)?;
                    }
                    "removefiles" => {
                        removal::exercise(registry, uid, &mut console)?;
                    }
                    "removeverify" => {
                        removal::verify(registry, uid)?;
                    }
                    "restoredoc" => {
                        replacement::restore(registry, uid)?;
                        console
                            .write("Restored all4096 original bytes with one checked commit\n")?;
                    }
                    "threads" => {
                        let h = unsafe { door_rt::thread::spawn(timer_entry, 7) }?;
                        check(
                            door_rt::thread::join(h, door_sys::time()? + 100)? == 7,
                            "interactive thread",
                        )?;
                        door_sys::close(h)?;
                        console
                            .write("New protected thread waited on timer, exited and joined\n")?;
                    }
                    "exit" => {
                        console.write("Closing session and flushing services.\n")?;
                        log(format_args!("DOOR_P08 CONSOLE COMMAND exit\n"));
                        return Ok(());
                    }
                    "" => {}
                    _ => {
                        console.write("Unknown command. Type help.\n")?;
                    }
                }
                log(format_args!("DOOR_P08 CONSOLE COMMAND {command}\n"));
                count = 0;
                console.write("door> ")?;
            }
            8 if count > 0 => {
                count -= 1;
                console.write("\u{8}")?;
            }
            32..=126 if count < input.len() => {
                input[count] = key;
                count += 1;
                let byte = [key];
                console.write(core::str::from_utf8(&byte).unwrap())?;
            }
            _ => {}
        }
    }
}
unsafe extern "C" fn timer_entry(argument: u64) -> ! {
    let code = match door_sys::time().and_then(|now| door_sys::sleep(now + 3)) {
        Ok(()) => argument as i64,
        Err(_) => -1,
    };
    door_rt::thread::finish(code)
}
unsafe extern "C" fn revocation_worker(mutex: u64) -> ! {
    if door_sys::wait(mutex, door_sys::time().unwrap_or(0) + 100).is_err() {
        door_rt::thread::finish(-261);
    }
    REVOCATION_WORKER_READY.store(1, Ordering::Release);
    let _ = door_sys::condition_wait(
        REVOCATION_CONDITION.load(Ordering::Acquire),
        mutex,
        door_sys::time().unwrap_or(0) + 1000,
    );
    // The supervisor must revoke the session while both threads are alive.
    // A normal return is a failing probe result, never a success substitute.
    door_rt::thread::finish(-260)
}
fn revocation_probe(startup: &StartupV2) -> Result<()> {
    STAGE.store(5, Ordering::Release);
    check(
        door_sys::identity(0)? == startup.identity,
        "revocation identity",
    )?;
    HEAP.initialize()?;
    let mut allocation = Vec::new();
    allocation
        .try_reserve_exact(32768)
        .map_err(|_| Error::Check("revocation allocation"))?;
    allocation.resize(32768, 0xacu8);
    let mut storage = Storage::connect(startup.base.inherited[inherited::REGISTRY].handle)?;
    let file = storage.open(
        path(startup.identity.uid)?,
        Disposition::OpenExisting,
        READ,
        SHARE_ALL,
    )?;
    let event = door_sys::wait_create(1, 0, 0)?;
    let mutex = door_sys::wait_create(2, 0, 0)?;
    let condition = door_sys::wait_create(3, 0, 0)?;
    REVOCATION_CONDITION.store(condition, Ordering::Release);
    let thread = unsafe { door_rt::thread::spawn(revocation_worker, mutex) }?;
    // Closing the waitable handle does not detach kernel ownership of a live
    // thread. Its completion event must survive through the final signal.
    door_sys::close(thread)?;
    let deadline = door_sys::time()? + 100;
    while REVOCATION_WORKER_READY.load(Ordering::Acquire) == 0 {
        let now = door_sys::time()?;
        check(now < deadline, "revocation worker readiness")?;
        door_sys::sleep(now + 1)?;
    }
    door_sys::sleep(door_sys::time()? + 2)?;
    let ready = door_service_protocol::Message::new(
        door_service_protocol::opcode::REVOCATION_READY,
        1,
        startup.identity.instance,
        Code::Success,
        &2u64.to_le_bytes(),
        1,
    )?;
    door_sys::channel_send(
        startup.base.inherited[inherited::CONTROL].handle,
        ready.bytes(),
        &[door_abi::Transfer {
            handle: mutex,
            rights: door_abi::rights::QUERY | door_abi::rights::WAIT | door_abi::rights::WRITE,
            mode: 0,
        }],
    )?;
    core::hint::black_box(&allocation);
    let _ = door_sys::wait(event, door_sys::time()? + 1000);
    storage.close(file)?;
    Err(Error::Check("revoked session survived"))
}
fn main(startup: StartupV2) -> i64 {
    if startup.mode == mode::DESKTOP {
        return window::run(startup);
    }
    if startup.mode == mode::WINDOW_TEST_HANG {
        return window::hang(startup);
    }
    let result = if startup.mode == mode::FAILURE {
        revocation_probe(&startup)
    } else {
        run(&startup)
    };
    match result {
        Ok(()) => {
            if startup.identity.uid == 1001 {
                71
            } else {
                72
            }
        }
        Err(error) => {
            fail(&error);
            -200 - STAGE.load(Ordering::Acquire) as i64
        }
    }
}
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    log(format_args!(
        "DOOR_P08 APP PANIC stage={}\n",
        STAGE.load(Ordering::Acquire)
    ));
    door_rt::panic()
}

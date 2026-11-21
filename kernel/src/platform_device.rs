//! Narrow kernel device adapters. The process layer checks the supplied grant;
//! these adapters additionally enforce the immutable manifest identity. No user
//! pointers reach the device owner and no filesystem policy executes here.
use crate::{arch, block_device::Session, console::Console};
use core::{
    arch::asm,
    cell::UnsafeCell,
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};
use door_abi::{
    Status,
    platform::{AudioInfo, BlockInfo, Identity, NetInfo, capability, manifest},
};
use door_boot_protocol::{BootInfo, Framebuffer};
use door_kernel::storage::{BlockDevice, Error, Gpt};
mod display;
pub use display::{
    capture_display_input, display_clock, display_gone, display_info, display_input,
    display_release, display_write, wall_time,
};

struct Owner {
    disk: Session,
    /// The optional NIC (P15), reached only by door-network.
    nic: Option<crate::net_device::NetSession>,
    /// The optional sound device (P15), reached only by door-audio.
    sound: Option<crate::sound_device::SoundSession>,
    writable: [(u64, u64); 2],
    terminal: Terminal,
    keyboard: door_kernel::keyboard::Decoder,
    pointer: door_kernel::pointer::Decoder,
    display: u64,
    /// `wall_time` anchor and RTC retry gate.
    wall: door_kernel::rtc::Wall,
}
struct State(UnsafeCell<Option<Owner>>);
unsafe impl Sync for State {}
static STATE: State = State(UnsafeCell::new(None));
static LOCK: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce(&mut Owner) -> Result<R, Status>) -> Result<R, Status> {
    let flags: u64;
    unsafe {
        asm!("pushfq","pop {}","cli",out(reg)flags);
    }
    // Device calls are bounded and never block on another process. The selected
    // P08 profile uses one CPU; IRQ exclusion prevents same-CPU lock inversion.
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let result = unsafe { (*STATE.0.get()).as_mut() }
        .ok_or(Status::DeviceLost)
        .and_then(f);
    LOCK.store(false, Ordering::Release);
    if flags & 0x200 != 0 {
        arch::enable_interrupts();
    }
    result
}
fn authority(identity: &Identity, manifest_id: u64, cap: u64) -> Result<(), Status> {
    if identity.manifest != manifest_id
        || identity.capabilities & cap != cap
        || identity.session != 0
    {
        Err(Status::AccessDenied)
    } else {
        Ok(())
    }
}
/// Called by the boot owner before any P08 service is runnable.
pub fn open(info: &BootInfo<'_>, framebuffer: Framebuffer) {
    // The machine may declare more CPUs (P18: a 2-vCPU desktop, and any
    // multi-core computer); the device profile runs on the boot CPU and the
    // others stay parked until the SMP scheduler (M0h).
    assert!(
        info.validate_platform()
            .expect("device platform")
            .processor_count
            >= 1,
        "P08 device profile requires a declared CPU"
    );
    assert_eq!(
        arch::online_mask(),
        1,
        "P08 polled device profile is single CPU"
    );
    arch::seal_single_cpu_profile().expect("seal device-profile CPU indexing");
    let mut disk = Session::open(info);
    let nic = crate::net_device::NetSession::open(info);
    let sound = crate::sound_device::SoundSession::open(info);
    let gpt = Gpt::read(&mut disk).expect("P08 validated private disk GPT");
    let home = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP07ExtGUID00")
        .expect("P08 home partition");
    let journal = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP07LogGUID00")
        .expect("P08 journal partition");
    let writable = [(home.first, home.sectors), (journal.first, journal.sectors)];
    let terminal = Terminal {
        screen: unsafe { Console::new(framebuffer) },
        rows: [[b' '; 80]; 28],
        column: 0,
        row: 0,
        theme: 0,
    };
    unsafe {
        assert!((*STATE.0.get()).is_none());
        *STATE.0.get() = Some(Owner {
            disk,
            nic,
            sound,
            writable,
            terminal,
            keyboard: door_kernel::keyboard::Decoder::new(),
            pointer: door_kernel::pointer::Decoder::new(),
            display: 0,
            wall: door_kernel::rtc::Wall::new(),
        });
    }
}
/// All user contexts and handles must have retired before releasing DMA.
pub fn close() {
    arch::disable_interrupts();
    assert!(!LOCK.load(Ordering::Acquire));
    let mut owner = unsafe { (*STATE.0.get()).take() }.expect("P08 device owner");
    if let Some(sound) = owner.sound.take() {
        let s = sound.statistics();
        sound.close();
        writeln!(
            unsafe { door_boot_debug::Serial::new() },
            "DOOR_P15 SOUND CLOSED played={} failed={} busy={}",
            s.played,
            s.failed,
            s.busy
        )
        .ok();
    }
    if let Some(nic) = owner.nic.take() {
        let s = nic.statistics();
        nic.close();
        writeln!(
            unsafe { door_boot_debug::Serial::new() },
            "DOOR_P15 NIC CLOSED sent={} received={} dropped={} busy={}",
            s.sent,
            s.received,
            s.dropped,
            s.busy
        )
        .ok();
    }
    let stats = owner.disk.statistics();
    owner.disk.close();
    writeln!(
        unsafe { door_boot_debug::Serial::new() },
        "DOOR_P08 DEVICE PASS reads={} writes={} flushes={} sampled_remaining={} reclaimed=8",
        stats.reads,
        stats.writes,
        stats.flushes,
        stats.sampled_remaining
    )
    .ok();
}
fn status(error: Error) -> Status {
    match error {
        Error::Unsupported => Status::Unsupported,
        Error::ReadOnly => Status::AccessDenied,
        _ => Status::IoFailure,
    }
}
fn transfer(lba: u64, bytes: usize) -> Result<u64, Status> {
    if bytes == 0 || bytes > 4096 || !bytes.is_multiple_of(512) {
        return Err(Status::InvalidArgument);
    }
    lba.checked_add((bytes / 512) as u64)
        .ok_or(Status::InvalidArgument)
}
pub fn block_info(identity: &Identity) -> Result<BlockInfo, Status> {
    authority(identity, manifest::STORAGE, capability::BLOCK)?;
    with(|owner| {
        let g = owner.disk.geometry();
        Ok(BlockInfo {
            sector_bytes: u64::from(g.sector_size),
            sectors: g.sectors,
            writable: !g.read_only,
            flush: g.flush_supported,
        })
    })
}
pub fn block_read(identity: &Identity, lba: u64, output: &mut [u8]) -> Result<(), Status> {
    authority(identity, manifest::STORAGE, capability::BLOCK)?;
    let end = transfer(lba, output.len())?;
    with(|owner| {
        if end > owner.disk.geometry().sectors {
            return Err(Status::InvalidArgument);
        }
        owner.disk.read_sectors(lba, output).map_err(status)
    })
}
pub fn block_write(identity: &Identity, lba: u64, input: &[u8]) -> Result<(), Status> {
    authority(identity, manifest::STORAGE, capability::BLOCK)?;
    let end = transfer(lba, input.len())?;
    with(|owner| {
        if !owner
            .writable
            .iter()
            .any(|(first, count)| lba >= *first && end <= first + count)
        {
            return Err(Status::AccessDenied);
        }
        owner.disk.write_sectors(lba, input).map_err(status)
    })
}
pub fn block_flush(identity: &Identity) -> Result<(), Status> {
    authority(identity, manifest::STORAGE, capability::BLOCK)?;
    with(|owner| owner.disk.flush().map_err(status))
}
pub fn net_info(identity: &Identity) -> Result<NetInfo, Status> {
    authority(identity, manifest::NETWORK, capability::NETWORK)?;
    with(|owner| {
        let nic = owner.nic.as_ref().ok_or(Status::NotFound)?;
        Ok(NetInfo {
            mac: nic.mac(),
            link: nic.link_up(),
        })
    })
}
pub fn net_send(identity: &Identity, frame: &[u8]) -> Result<(), Status> {
    authority(identity, manifest::NETWORK, capability::NETWORK)?;
    with(|owner| {
        let nic = owner.nic.as_mut().ok_or(Status::NotFound)?;
        nic.send(frame).map_err(|e| match e {
            door_kernel::drivers::Error::QueueFull => Status::WouldBlock,
            door_kernel::drivers::Error::Bounds => Status::InvalidArgument,
            door_kernel::drivers::Error::Removed => Status::DeviceLost,
            _ => Status::IoFailure,
        })
    })
}
/// The next received frame's length (0 when none waits).
pub fn net_receive(identity: &Identity, out: &mut [u8]) -> Result<usize, Status> {
    authority(identity, manifest::NETWORK, capability::NETWORK)?;
    with(|owner| {
        let nic = owner.nic.as_mut().ok_or(Status::NotFound)?;
        // A malformed completion is dropped by the driver; report nothing.
        match nic.receive(out) {
            Ok(Some(n)) => Ok(n),
            Ok(None) | Err(door_kernel::drivers::Error::Malformed) => Ok(0),
            Err(door_kernel::drivers::Error::Removed) => Err(Status::DeviceLost),
            Err(_) => Err(Status::IoFailure),
        }
    })
}
fn audio_status(e: door_kernel::drivers::Error) -> Status {
    match e {
        door_kernel::drivers::Error::QueueFull => Status::WouldBlock,
        door_kernel::drivers::Error::Bounds => Status::InvalidArgument,
        door_kernel::drivers::Error::Removed => Status::DeviceLost,
        _ => Status::IoFailure,
    }
}
pub fn audio_info(identity: &Identity) -> Result<AudioInfo, Status> {
    authority(identity, manifest::AUDIO, capability::AUDIO)?;
    with(|owner| {
        let sound = owner.sound.as_mut().ok_or(Status::NotFound)?;
        let queued = sound.queued().map_err(audio_status)?;
        let s = sound.statistics();
        Ok(AudioInfo {
            rate: door_kernel::drivers::virtio_sound::RATE,
            channels: door_kernel::drivers::virtio_sound::CHANNELS,
            running: sound.running(),
            queued: queued as u8,
            capacity: door_kernel::drivers::virtio_sound::PERIODS as u8,
            played: s.played,
            failed: s.failed as u32,
            capture: sound.capture_state(),
        })
    })
}
pub fn audio_write(identity: &Identity, period: &[u8]) -> Result<(), Status> {
    authority(identity, manifest::AUDIO, capability::AUDIO)?;
    with(|owner| {
        let sound = owner.sound.as_mut().ok_or(Status::NotFound)?;
        sound.write(period).map_err(audio_status)
    })
}
/// The next captured period into `period`; false while none waits.
pub fn audio_read(identity: &Identity, period: &mut [u8]) -> Result<bool, Status> {
    authority(identity, manifest::AUDIO, capability::AUDIO)?;
    with(|owner| {
        let sound = owner.sound.as_mut().ok_or(Status::NotFound)?;
        sound.capture_read(period).map_err(audio_status)
    })
}
pub fn audio_control(identity: &Identity, command: u64) -> Result<(), Status> {
    authority(identity, manifest::AUDIO, capability::AUDIO)?;
    with(|owner| {
        let sound = owner.sound.as_mut().ok_or(Status::NotFound)?;
        match command {
            door_abi::platform::AUDIO_START => sound.start(),
            door_abi::platform::AUDIO_STOP => sound.stop(),
            door_abi::platform::AUDIO_CAPTURE_START => sound.capture_start(),
            door_abi::platform::AUDIO_CAPTURE_STOP => sound.capture_stop(),
            _ => return Err(Status::InvalidArgument),
        }
        .map_err(audio_status)
    })
}
pub fn console_write(identity: &Identity, input: &[u8]) -> Result<usize, Status> {
    authority(identity, manifest::CONSOLE, capability::CONSOLE)?;
    if input.len() > 1024 || core::str::from_utf8(input).is_err() {
        return Err(Status::InvalidArgument);
    }
    with(|owner| {
        if owner.display == 0 {
            owner.terminal.write(input)
        } else if let Ok(text) = core::str::from_utf8(input) {
            door_boot_debug::console_record(&mut unsafe { door_boot_debug::Serial::new() }, text)
                .ok();
        }
        Ok(input.len())
    })
}
pub fn console_read(identity: &Identity) -> Result<u64, Status> {
    authority(identity, manifest::CONSOLE, capability::CONSOLE)?;
    with(|owner| {
        if owner.display != 0 {
            Err(Status::WouldBlock)
        } else {
            match key(&mut owner.keyboard) {
                Some(byte) => Ok(u64::from(byte)),
                None => Err(Status::WouldBlock),
            }
        }
    })
}
struct Terminal {
    screen: Console,
    rows: [[u8; 80]; 28],
    column: usize,
    row: usize,
    theme: u64,
}
impl Terminal {
    fn newline(&mut self) {
        self.column = 0;
        if self.row == 27 {
            self.rows.rotate_left(1);
            self.rows[27] = [b' '; 80];
        } else {
            self.row += 1;
        }
    }
    fn write(&mut self, input: &[u8]) {
        for &byte in input {
            match byte {
                b'\n' => self.newline(),
                b'\r' => self.column = 0,
                8 if self.column > 0 => {
                    self.column -= 1;
                    self.rows[self.row][self.column] = b' ';
                }
                32..=126 => {
                    self.rows[self.row][self.column] = byte;
                    self.column += 1;
                    if self.column == 80 {
                        self.newline();
                    }
                }
                _ => {}
            }
        }
        self.screen
            .terminal_theme(&self.rows, self.column, self.row, self.theme);
        let mut serial = unsafe { door_boot_debug::Serial::new() };
        if let Ok(text) = core::str::from_utf8(input) {
            door_boot_debug::console_record(&mut serial, text).ok();
        }
    }
}
fn key(decoder: &mut door_kernel::keyboard::Decoder) -> Option<u8> {
    for _ in 0..32 {
        let ready: u8;
        unsafe {
            asm!("in al, dx",in("dx")0x64u16,out("al")ready,options(nomem,nostack));
        }
        if ready & 1 == 0 {
            return None;
        }
        let scan: u8;
        unsafe {
            asm!("in al, dx",in("dx")0x60u16,out("al")scan,options(nomem,nostack));
        }
        if ready & 0x20 != 0 {
            continue;
        }
        if let Some(byte) = decoder.feed(scan) {
            return Some(byte);
        }
    }
    None
}

pub fn console_control(identity: &Identity, command: u64, value: u64) -> Result<u64, Status> {
    authority(identity, manifest::CONSOLE, capability::CONSOLE)?;
    with(|owner| {
        match command {
            0 if value == 0 => {
                owner.terminal.rows = [[b' '; 80]; 28];
                owner.terminal.column = 0;
                owner.terminal.row = 0;
            }
            1 if value <= 2 => owner.terminal.theme = value,
            _ => return Err(Status::InvalidArgument),
        }
        if owner.display == 0 {
            owner.terminal.write(&[]);
        }
        Ok(0)
    })
}

pub fn monitor_io() -> Result<(u64, u64, u64), Status> {
    with(|owner| {
        let s = owner.disk.statistics();
        Ok((s.reads, s.writes, s.flushes))
    })
}

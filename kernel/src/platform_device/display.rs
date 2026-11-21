//! Sole display grant and polled i8042 backend. No client can map the framebuffer.
use super::*;
use core::sync::atomic::AtomicU64;
use door_abi::graphics as g;
static INPUT_OWNER: AtomicU64 = AtomicU64::new(0);
static RAW: door_kernel::input::RawQueue = door_kernel::input::RawQueue::new();
/// Called by the BSP timer with IRQs disabled. It never takes device, VM or
/// scheduler locks, and never allocates. Text-console polling remains unchanged.
pub fn capture_display_input(time: u64) {
    if INPUT_OWNER.load(Ordering::Acquire) == 0 {
        return;
    }
    for _ in 0..64 {
        let status = port_read(0x64);
        if status & 1 == 0 {
            break;
        }
        let byte = port_read(0x60);
        let _ = RAW.push((u16::from(status) << 8) | u16::from(byte), time);
    }
}
fn port_read(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al,dx",in("dx")port,out("al")value,options(nomem,nostack)) }
    value
}
fn port_write(port: u16, value: u8) {
    unsafe { asm!("out dx,al",in("dx")port,in("al")value,options(nomem,nostack)) }
}
fn command(port: u16, value: u8) -> Result<(), Status> {
    for _ in 0..100_000 {
        if port_read(0x64) & 2 == 0 {
            port_write(port, value);
            return Ok(());
        }
        core::hint::spin_loop()
    }
    Err(Status::Timeout)
}
/// Next auxiliary-port byte. Keyboard bytes arriving during initialization
/// are discarded, exactly as the original enable sequence did.
fn mouse_read() -> Result<u8, Status> {
    for _ in 0..100_000 {
        let status = port_read(0x64);
        if status & 1 != 0 {
            let value = port_read(0x60);
            if status & 32 != 0 {
                return Ok(value);
            }
        }
        core::hint::spin_loop()
    }
    Err(Status::Timeout)
}
/// One mouse command byte, acknowledged with 0xFA.
fn mouse_write(value: u8) -> Result<(), Status> {
    command(0x64, 0xd4)?;
    command(0x60, value)?;
    if mouse_read()? != 0xfa {
        return Err(Status::IoFailure);
    }
    Ok(())
}
/// Returns the negotiated packet protocol. The IntelliMouse sequence (sample
/// rates 200, 100, 80, then GET ID) switches a wheel mouse to ID 3 and
/// four-byte packets; a device that refuses it stays a standard mouse.
fn mouse_enable() -> Result<door_kernel::pointer::Protocol, Status> {
    // Initialization happens before desktop input is enabled. Existing keyboard
    // bytes are drained only here; normal polling preserves every make/break.
    for _ in 0..64 {
        if port_read(0x64) & 1 == 0 {
            break;
        }
        let _ = port_read(0x60);
    }
    command(0x64, 0xa8)?;
    mouse_write(0xf6)?;
    let probe = (|| {
        for rate in [200, 100, 80] {
            mouse_write(0xf3)?;
            mouse_write(rate)?;
        }
        mouse_write(0xf2)?;
        mouse_read()
    })();
    let protocol = match probe {
        Ok(id) => door_kernel::pointer::Protocol::from_device_id(id),
        // A NAK means the device rejected the probe; it remains standard.
        Err(Status::IoFailure) => door_kernel::pointer::Protocol::Standard,
        Err(error) => return Err(error),
    };
    // Restore the default 100 reports/s; this does not leave IntelliMouse mode.
    mouse_write(0xf3)?;
    mouse_write(100)?;
    mouse_write(0xf4)?;
    Ok(protocol)
}
static WHEEL_PROTOCOL: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
fn pointer_decoder() -> door_kernel::pointer::Decoder {
    door_kernel::pointer::Decoder::with_protocol(match WHEEL_PROTOCOL.load(Ordering::Acquire) {
        3 => door_kernel::pointer::Protocol::Wheel,
        4 => door_kernel::pointer::Protocol::Explorer,
        _ => door_kernel::pointer::Protocol::Standard,
    })
}
/// Host display-scale hint (percent, 0 = none), read once from QEMU fw_cfg
/// `opt/door/scale` when the display is first acquired.
static SCALE_HINT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SCALE_HINT_READ: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Read-only fw_cfg query, only after the "QEMU" signature confirms the
/// device (other platforms report no hint). Runs with IRQs off under the
/// device guard; fw_cfg ports have no other kernel user after boot.
fn read_scale_hint() -> u32 {
    let mut signature = [0u8; 4];
    unsafe {
        asm!("out dx, ax", in("dx") 0x510u16, in("ax") 0u16, options(nomem, nostack, preserves_flags));
    }
    for byte in &mut signature {
        *byte = port_read(0x511);
    }
    if &signature != b"QEMU" {
        return 0;
    }
    let mut text = [0u8; 8];
    // SAFETY: exclusive fw_cfg port use (see above); bounded output buffer.
    match unsafe { door_boot_debug::fw_cfg("opt/door/scale", &mut text) } {
        Some(length) => g::scale_hint_from_text(&text[..length]),
        None => 0,
    }
}
fn own(owner: &Owner, identity: &Identity) -> Result<(), Status> {
    if owner.display != identity.pid {
        Err(Status::AccessDenied)
    } else {
        Ok(())
    }
}
pub fn display_info(identity: &Identity) -> Result<g::DisplayInfo, Status> {
    authority(identity, manifest::WINDOW, capability::DISPLAY)?;
    with(|owner| {
        if owner.display != 0 && owner.display != identity.pid {
            return Err(Status::AccessDenied);
        }
        let fb = owner.terminal.screen.framebuffer();
        if fb.width > 3840 || fb.height > 2160 {
            return Err(Status::Unsupported);
        }
        if owner.display == 0 {
            let protocol = mouse_enable()?;
            WHEEL_PROTOCOL.store(
                match protocol {
                    door_kernel::pointer::Protocol::Standard => 0,
                    door_kernel::pointer::Protocol::Wheel => 3,
                    door_kernel::pointer::Protocol::Explorer => 4,
                },
                Ordering::Release,
            );
            RAW.reset_quiescent();
            owner.keyboard = door_kernel::keyboard::Decoder::new();
            owner.pointer = pointer_decoder();
            owner.display = identity.pid;
            INPUT_OWNER.store(identity.pid, Ordering::Release);
        }
        let wheel = if owner.pointer.protocol().has_wheel() {
            g::WHEEL
        } else {
            0
        };
        if !SCALE_HINT_READ.swap(true, Ordering::AcqRel) {
            SCALE_HINT.store(read_scale_hint(), Ordering::Release);
        }
        Ok(g::DisplayInfo {
            width: fb.width,
            height: fb.height,
            stride: fb.stride,
            format: fb.format,
            capabilities: g::CPU_COPY | g::PARTIAL_COPY | g::POINTER | wheel,
            max_copy: g::MAX_COPY_BYTES as u64,
        }
        .with_scale_hint(SCALE_HINT.load(Ordering::Acquire)))
    })
}
pub fn display_write(identity: &Identity, first: u64, pixels: &[u8]) -> Result<(), Status> {
    authority(identity, manifest::WINDOW, capability::DISPLAY)?;
    with(|owner| {
        own(owner, identity)?;
        owner.terminal.screen.present_pixels(first, pixels)
    })
}
pub fn display_input(identity: &Identity) -> Result<g::Input, Status> {
    authority(identity, manifest::WINDOW, capability::DISPLAY)?;
    with(|owner| {
        own(owner, identity)?;
        for _ in 0..64 {
            let next = RAW.pop();
            if next.is_err() {
                // This syscall holds the BSP device guard with IRQs off, so
                // both queue endpoints are quiescent during epoch reset.
                RAW.reset_quiescent();
                owner.keyboard = door_kernel::keyboard::Decoder::new();
                owner.pointer = pointer_decoder();
                return Ok(g::Input {
                    kind: g::RESET,
                    ..g::Input::default()
                });
            }
            let Some((raw, time)) = next.unwrap() else {
                return Err(Status::WouldBlock);
            };
            let status = (raw >> 8) as u8;
            let byte = raw as u8;
            if status & 0xc0 != 0 {
                RAW.reset_quiescent();
                owner.keyboard = door_kernel::keyboard::Decoder::new();
                owner.pointer = pointer_decoder();
                return Ok(g::Input {
                    kind: g::RESET,
                    ..g::Input::default()
                });
            }
            let event = if status & 32 != 0 {
                owner.pointer.feed(byte)
            } else {
                owner.keyboard.event(byte)
            };
            if let Some(mut event) = event {
                event.timestamp = time;
                return Ok(event);
            }
        }
        Err(Status::WouldBlock)
    })
}
pub fn display_release(identity: &Identity) -> Result<(), Status> {
    authority(identity, manifest::WINDOW, capability::DISPLAY)?;
    with(|owner| {
        own(owner, identity)?;
        INPUT_OWNER.store(0, Ordering::Release);
        RAW.reset_quiescent();
        owner.display = 0;
        owner.keyboard = door_kernel::keyboard::Decoder::new();
        owner.terminal.write(b"");
        Ok(())
    })
}
pub fn display_gone(pid: u64) {
    let _ = with(|owner| {
        if owner.display == pid {
            INPUT_OWNER.store(0, Ordering::Release);
            RAW.reset_quiescent();
            owner.display = 0;
            owner.keyboard = door_kernel::keyboard::Decoder::new();
            owner.terminal.write(b"");
        }
        Ok(())
    });
}

/// Read-only civil time on the declared QEMU RTC profile. All indexed accesses
/// are serialized by the BSP device guard with IRQs disabled. Port70 reads return
/// FF on QEMU, so no fictitious saved NMI value is read back; writes keep NMI on.
/// RegC is never read and no CMOS/NVRAM data register is written.
pub fn display_clock(identity: &Identity) -> Result<door_abi::clock::WallClock, Status> {
    authority(identity, manifest::WINDOW, capability::DISPLAY)?;
    with(|owner| {
        own(owner, identity)?;
        sample_rtc(1)
    })
}
/// Two identical samples between update cycles with VRT (register D bit 7,
/// valid RAM and time) set. `waits` bounds the polls for UIP to clear.
/// Caller holds the device guard.
fn sample_rtc(waits: u32) -> Result<door_abi::clock::WallClock, Status> {
    fn read(index: u8) -> u8 {
        port_write(0x70, index);
        port_read(0x71)
    }
    let sample = || {
        [
            read(0),
            read(2),
            read(4),
            read(7),
            read(8),
            read(9),
            read(0x32),
            read(0x0b),
        ]
    };
    for _ in 0..3 {
        let mut clear = false;
        for _ in 0..waits {
            if read(0x0a) & 0x80 == 0 {
                clear = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !clear {
            return Err(Status::WouldBlock);
        }
        let first = sample();
        let second = sample();
        if read(0x0a) & 0x80 == 0 && first == second {
            if read(0x0d) & 0x80 == 0 {
                return Err(Status::Unsupported);
            }
            return door_kernel::rtc::decode(first).ok_or(Status::IoFailure);
        }
    }
    Err(Status::WouldBlock)
}
/// UIP polls per `wall_time` sample with interrupts off (tens of
/// microseconds). UIP stays set for up to ~2 ms around each once-a-second
/// update; a longer wait is deferred to the next tick, not spun on.
const WALL_UIP_POLLS: u32 = 64;
/// `operation::WALL_TIME` for every process: the anchored RTC reading plus
/// the tick offset (`door_kernel::rtc::Wall`), re-read once a minute. A
/// cleared VRT bit or an undecodable/out-of-range reading drops the anchor
/// and reports Unsupported; no time is invented. Failed samples are retried
/// at most once per tick (update in progress) or second (invalid clock).
/// Same port discipline as `display_clock`, no display ownership required
/// (read-only RTC access).
pub fn wall_time(tick: u64) -> Result<u64, Status> {
    with(|owner| {
        owner.wall.read(tick, || {
            sample_rtc(WALL_UIP_POLLS)?
                .to_filetime()
                .ok_or(Status::Unsupported)
        })
    })
    .map_err(|e| {
        if e == Status::DeviceLost {
            Status::Unsupported
        } else {
            e
        }
    })
}

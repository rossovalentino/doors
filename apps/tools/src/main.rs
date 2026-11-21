#![no_std]
#![no_main]
extern crate alloc;
use alloc::{format, string::String};
use core::fmt::Write;
use door_abi::platform::{StartupV2, inherited};
use door_service_client::{Console, Disposition, READ, SHARE_ALL, Storage, WRITE};
use door_userland::{arguments, cwd, environment, print, resolve};

#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);

fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize().is_err() {
        return 70;
    }
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    // A Terminal tab's program writes to the tab's console.
    let mut console = match Console::connect_for(
        registry,
        &startup.environment[..usize::from(startup.environment_bytes)],
    ) {
        Ok(c) => c,
        Err(_) => return 71,
    };
    let mut storage = match Storage::connect(registry) {
        Ok(s) => s,
        Err(_) => return 72,
    };
    let args = arguments(&startup);
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    let result = (|| -> Result<i64, door_service_client::Error> {
        match command {
            "help" => print(
                &mut console,
                "tools: echo, args, env, wc FILE, sum FILE, repeat FILE TEXT COUNT, sleep MILLISECONDS, true, false\n",
            )?,
            "echo" => print(&mut console, &format!("{}\n", args[2..].join(" ")))?,
            "args" => {
                for (i, value) in args.iter().enumerate() {
                    print(&mut console, &format!("argv[{i}]={value}\n"))?;
                }
            }
            "env" => {
                for value in environment(&startup) {
                    print(&mut console, &format!("{value}\n"))?;
                }
            }
            "true" => return Ok(0),
            "repeat" => {
                let path = resolve(
                    &cwd(&startup),
                    args.get(2).ok_or(door_service_client::Error::Malformed)?,
                )
                .map_err(|_| door_service_client::Error::Malformed)?;
                let text = args.get(3).ok_or(door_service_client::Error::Malformed)?;
                let count = args
                    .get(4)
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or(door_service_client::Error::Malformed)?;
                if text
                    .len()
                    .checked_add(1)
                    .and_then(|n| n.checked_mul(count))
                    .is_none_or(|n| n > 512 * 1024)
                {
                    return Err(door_service_client::Error::Malformed);
                }
                let data = format!("{text}\n").repeat(count);
                let file = storage.open(&path, Disposition::CreateAlways, WRITE, SHARE_ALL)?;
                let written = storage.write_all(&file, 0, data.as_bytes());
                let closed = storage.close(file);
                written.and(closed)?;
            }
            "false" => return Ok(1),
            "sleep" => {
                let millis = args
                    .get(2)
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or(door_service_client::Error::Malformed)?;
                let ticks = door_abi::platform::ticks_from_millis(millis)
                    .ok_or(door_service_client::Error::Malformed)?;
                let deadline = door_sys::time()?
                    .checked_add(ticks)
                    .ok_or(door_service_client::Error::Malformed)?;
                door_sys::sleep(deadline)?;
            }
            "wc" | "sum" => {
                let name = args.get(2).ok_or(door_service_client::Error::Malformed)?;
                let path = resolve(&cwd(&startup), name)
                    .map_err(|_| door_service_client::Error::Malformed)?;
                let file = storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL)?;
                let mut offset = 0;
                let mut lines = 0u64;
                let mut words = 0u64;
                let mut in_word = false;
                let mut checksum = 0xcbf2_9ce4_8422_2325u64;
                let mut buffer = [0; 192];
                loop {
                    let count = storage.read(&file, offset, &mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    for &byte in &buffer[..count] {
                        if byte == b'\n' {
                            lines += 1;
                        }
                        let space = byte.is_ascii_whitespace();
                        if !space && !in_word {
                            words += 1;
                        }
                        in_word = !space;
                        checksum = (checksum ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
                    }
                    offset += count as u64;
                }
                storage.close(file)?;
                let mut text = String::new();
                if command == "wc" {
                    let _ = writeln!(text, "{lines} {words} {offset} {name}");
                } else {
                    let _ = writeln!(text, "{checksum:016x} {name}");
                }
                print(&mut console, &text)?;
            }
            _ => {
                print(&mut console, "Unknown tools command. Run tools help.\n")?;
                return Ok(2);
            }
        }
        Ok(0)
    })();
    match result {
        Ok(code) => code,
        Err(error) => {
            let _ = print(&mut console, &format!("tools: {error:?}\n"));
            1
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

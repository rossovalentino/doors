#![no_std]
#![no_main]
//! The inbox apps' image: the kernel-authenticated launch manifest picks the
//! app (`door_inbox::App::from_manifest`).
use door_abi::platform::StartupV2;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(80 * 1024 * 1024).is_err() {
        return -240;
    }
    let result = match door_inbox::App::from_manifest(startup.identity.manifest) {
        Some(door_inbox::App::Clock) => {
            let registry = startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle;
            door_app_ui::run(&startup, &mut door_inbox::clock::Clock::new(registry))
        }
        Some(door_inbox::App::Weather) => {
            let registry = startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle;
            door_app_ui::run(
                &startup,
                &mut door_inbox::weather::Weather::new().with_registry(registry),
            )
        }
        Some(door_inbox::App::Magnifier) => {
            door_app_ui::run(&startup, &mut door_inbox::access::Magnifier::new())
        }
        Some(door_inbox::App::Narrator) => {
            door_app_ui::run(&startup, &mut door_inbox::access::Narrator::new())
        }
        Some(door_inbox::App::OnScreenKeyboard) => {
            door_app_ui::run(&startup, &mut door_inbox::access::Keyboard::new())
        }
        Some(door_inbox::App::VoiceAccess) => {
            door_app_ui::run(&startup, &mut door_inbox::access::VoiceAccess::new())
        }
        Some(door_inbox::App::SnippingTool) => door_app_ui::run(
            &startup,
            &mut door_inbox::snipping::Snipping::new(startup.identity.uid),
        ),
        Some(door_inbox::App::Paint) => door_app_ui::run(
            &startup,
            &mut door_inbox::paint::Paint::new(startup.identity.uid),
        ),
        Some(door_inbox::App::Calculator) => {
            door_app_ui::run(&startup, &mut door_inbox::calculator::Calculator::new())
        }
        None => return -242,
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_INBOX FAILED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}

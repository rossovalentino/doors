#![no_std]
#![no_main]
//! `door-assist`: the caption-panel assistive client (guest binary).
use door_abi::platform::StartupV2;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(48 * 1024 * 1024).is_err() {
        return -240;
    }
    // App text reaches the serial line only on the diagnostics image.
    let diagnostics = startup.mode == door_service_protocol::accessibility::ASSIST_DIAGNOSTIC_MODE;
    match door_ui_native::run(&startup, door_gallery::assist::Assist::new(diagnostics)) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_ASSIST FAILED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}

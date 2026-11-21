#![no_std]
#![no_main]
//! The Door UI component gallery (guest binary).
use door_abi::platform::StartupV2;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(96 * 1024 * 1024).is_err() {
        return -240;
    }
    match door_ui_native::run(&startup, door_gallery::Gallery::new()) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_GALLERY FAILED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}

#![no_std]
#![no_main]
use door_abi::platform::StartupV2;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(128 * 1024 * 1024).is_err() {
        return -240;
    }
    let registry = startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle;
    let mut app = door_preview::Preview::new(startup.identity.uid).with_registry(registry);
    match door_app_ui::run(&startup, &mut app) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_PREVIEW FAILED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}

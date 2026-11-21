//! `door-search`: Init's windowless Start search host (Workspace manifest,
//! search-host mode). An index over the session's files and no UI, so it
//! links neither fonts nor icons.
#![no_std]
#![no_main]
extern crate alloc;
use door_abi::platform::StartupV2;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: StartupV2) -> i64 {
    if let Err(error) = HEAP.initialize_with_bytes(32 * 1024 * 1024) {
        door_rt::diagnostic(format_args!("DOOR_P12 HEAP FAILED {error:?}\n"));
        return -240;
    }
    match door_workspace::shell_search::host(&startup) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_P12 SEARCH_HOST STOPPED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}

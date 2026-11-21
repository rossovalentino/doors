#![no_std]
#![no_main]
extern crate alloc;
use door_system_services::{credentials, session};
/// The session service's heap holds only the Argon2 working memory (P14-LOGON).
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: door_abi::platform::StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(session::HEAP_BYTES).is_err() {
        return -133;
    }
    let Ok(blocks) = credentials::PARAMS.blocks() else {
        return -134;
    };
    // Allocated once for the process lifetime; wiped after every derivation.
    let memory = alloc::vec![argon2::Block::new(); blocks].leak();
    session::run(startup, memory)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

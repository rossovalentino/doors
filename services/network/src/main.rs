#![no_std]
#![no_main]
extern crate alloc;
/// door-network's heap: socket buffers and the TCP/IP stack's tables.
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: door_abi::platform::StartupV2) -> i64 {
    if HEAP
        .initialize_with_bytes(door_network::HEAP_BYTES)
        .is_err()
    {
        return -150;
    }
    door_network::run(startup)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

#![no_std]
#![no_main]
extern crate alloc;
/// door-audio's heap: the mixer and stream bookkeeping (PCM itself arrives
/// in mapped sections).
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
fn main(startup: door_abi::platform::StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(door_audio::HEAP_BYTES).is_err() {
        return -150;
    }
    door_audio::run(startup)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

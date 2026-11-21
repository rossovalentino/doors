#![no_std]
#![no_main]
door_rt::entry_v2!(door_system_services::console::run);
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

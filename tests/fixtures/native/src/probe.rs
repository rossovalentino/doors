#![no_std]
#![no_main]
// Deliberately no ABI/runtime imports. Assembly literals independently exercise
// the published binary boundary rather than sharing wrapper/register definitions.
core::arch::global_asm!(include_str!("probe.S"));
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
}

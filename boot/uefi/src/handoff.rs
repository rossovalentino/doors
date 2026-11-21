use core::arch::naked_asm;
/// # Safety
/// CR3 names initialized owned tables; all transition code, BootInfo, kernel and
/// guarded stack mappings are present. NX support was checked. Interrupts stay off.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn enter(
    entry: u64,
    boot: u64,
    length: u64,
    stack: u64,
    root: u64,
) -> ! {
    naked_asm!(
        "cli",
        "mov r9, rdi",
        "mov r10, rsi",
        "mov r11, rdx",
        "mov r12, rcx",
        "mov r13, r8",
        "mov ecx, 0xc0000080",
        "rdmsr",
        "or eax, 0x800",
        "wrmsr",
        "mov rax, cr0",
        "or rax, 0x10000",
        "mov cr0, rax",
        "mov cr3, r13",
        "mov rsp, r12",
        "and rsp, -16",
        "xor ebp, ebp",
        "mov rdi, r10",
        "mov rsi, r11",
        "cld",
        "jmp r9"
    );
}

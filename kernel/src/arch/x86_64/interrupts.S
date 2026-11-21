/* Intel SDM 325384-081US 6.14: hardware always saves SS:RSP in long mode.
 * All gates are ring-0 interrupt gates. Error-code vectors receive the CPU
 * error; every other stub adds zero. This is an original Door entry path.
 */
.section .text,"ax"
.macro TRAP number
.global door_vector_\number
.type door_vector_\number,@function
door_vector_\number:
.if (\number != 8) && (\number != 10) && (\number != 11) && (\number != 12) && (\number != 13) && (\number != 14) && (\number != 17) && (\number != 21) && (\number != 29) && (\number != 30)
    pushq $0
.endif
    pushq $\number
    jmp door_interrupt_common
.endm

.altmacro
.set vector, 0
.rept 256
TRAP %vector
.set vector, vector + 1
.endr
.noaltmacro

.global door_syscall_entry
.type door_syscall_entry,@function
door_syscall_entry:
    /* FMASK has cleared IF/TF/DF/AC. No stack access precedes this swap and
     * acquisition of the trusted per-CPU stack. Native FS is never used here. */
    swapgs
    movq %rsp, %gs:16
    movq %gs:0, %rsp
    pushq ${user_data_selector}
    pushq %gs:16
    pushq %r11
    pushq ${user_code_selector}
    pushq %rcx
    pushq $0
    pushq $0x100
    jmp door_interrupt_common

/* SYSCALL from compatibility mode (CSTAR, AMD CPUs): not a Door entry. The
 * 32-bit program sees an invalid-opcode fault at the following instruction;
 * RCX holds that 32-bit return address and R11 its flags. */
.global door_syscall_compat_entry
.type door_syscall_compat_entry,@function
door_syscall_compat_entry:
    swapgs
    movq %rsp, %gs:16
    movq %gs:0, %rsp
    pushq ${user_data_selector}
    pushq %gs:16
    pushq %r11
    pushq ${user_code32_selector}
    pushq %rcx
    pushq $0
    pushq $6
    jmp door_interrupt_common

door_interrupt_common:
    cld
    pushq %rax
    pushq %rbx
    pushq %rcx
    pushq %rdx
    pushq %rbp
    pushq %rsi
    pushq %rdi
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, %r12
    /* Keep the interrupted x87/MMX/SSE state before any compiler code.
     * CR4.OSXSAVE is disabled; AVX and further XSAVE state are not enabled.
     * R12 is preserved by the SysV handler, and its old value is in the frame.
     */
    subq $560, %rsp
    andq $-16, %rsp
    fxsave64 (%rsp)
    movq %r12, 512(%rsp)
    movl $0xc0000100, %ecx
    rdmsr
    shlq $32, %rdx
    orq %rdx, %rax
    movq %rax, 520(%rsp)
    movq %cr3, %rax
    movq %rax, 528(%rsp)
    /* Detect the actual GS domain, not only saved CS: an NMI can interrupt
     * either side of a SWAPGS while CS is still0. FSGSBASE is disabled and user
     * GS bases are constrained to the low half. Restore the domain on return. */
    xorq %r13, %r13
    movl $0xc0000101, %ecx
    rdmsr
    testl %edx, %edx
    js 3f
    swapgs
    movq $1, %r13
3:
    testb $3, 144(%r12)
    jz 4f
    movq $1, %r13
4:
    movq %r13, 544(%rsp)
    movq %gs:0, %rax
    movq %rax, 536(%rsp)
    movl $0xc0000102, %ecx
    rdmsr
    shlq $32, %rdx
    orq %rdx, %rax
    movq %rax, 552(%rsp)
    movq %gs:8, %rax
    movq %rax, %cr3
    movq %rsp, %rdi
    movq %cr2, %rsi
    call door_trap_dispatch
    /* Commit outgoing-stack retirement only after changing RSP. The call uses
     * free stack below the selected context; it never touches the old stack.
     */
    movq %rax, %rsp
    movq %rsp, %r12
    movq %rsp, %rdi
    call door_switch_commit
    movq %r12, %rsp
    /* DS, ES and FS are not part of a saved context, and a thread may load
     * any selector into them (a null one, or the 32-bit code selector): every
     * return reloads the flat user data selector, so no thread's choice
     * reaches the next one on this CPU (compatibility-mode code faults on a
     * null segment). FS's base is written after its selector, which resets
     * it. GS stays as it is: its bases are the SWAPGS pair. */
    movw ${user_data_selector}, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %fs
    movq 520(%rsp), %rax
    movq %rax, %rdx
    shrq $32, %rdx
    movl $0xc0000100, %ecx
    wrmsr
    movq 552(%rsp), %rax
    movq %rax, %rdx
    shrq $32, %rdx
    movl $0xc0000102, %ecx
    wrmsr
    fxrstor64 (%rsp)
    movq 528(%rsp), %rax
    movq %rax, %cr3
    cmpq $0, 544(%rsp)
    je 5f
    swapgs
5:
    movq 512(%rsp), %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rdi
    popq %rsi
    popq %rbp
    popq %rdx
    popq %rcx
    popq %rbx
    popq %rax
    addq $16, %rsp
    iretq

.global door_load_segments
.type door_load_segments,@function
door_load_segments:
    pushq ${code_selector}
    leaq 1f(%rip), %rax
    pushq %rax
    lretq
1:
    movw ${data_selector}, %ax
    movw %ax, %ss
    /* DS, ES and FS hold the flat DPL-3 user data selector (64-bit code
     * ignores them): IRET to ring 3 would null a ring-0 one, and a 32-bit
     * (compatibility-mode) program faults on any access through a null
     * segment. FS's base comes from its MSR afterwards (the TEB32 of a
     * 32-bit thread); GS stays null, its bases being the SWAPGS pair. */
    movw ${user_data_selector}, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %fs
    xor %eax, %eax
    movw %ax, %gs
    retq

.global door_breakpoint_probe
.type door_breakpoint_probe,@function
door_breakpoint_probe:
    movdqu door_simd_pattern(%rip), %xmm0
    movabsq $0x1a2b3c4d5e6f7788, %r10
    int3
    pcmpeqb door_simd_pattern(%rip), %xmm0
    pmovmskb %xmm0, %eax
    cmpq $65535, %rax
    jne 2f
    movabsq $0x1a2b3c4d5e6f7788, %rax
    cmpq %rax, %r10
    sete %al
    movzbl %al, %eax
    retq
2:  xor %eax, %eax
    retq

.section .rodata,"a"
.balign 16
door_simd_pattern:
    .quad 0x91e3a257db068c4f, 0x3254d6f890abcdef

.macro VECTOR_ADDRESS number
    .quad door_vector_\number
.endm
.global door_vector_addresses
door_vector_addresses:
.altmacro
.set vector, 0
.rept 256
VECTOR_ADDRESS %vector
.set vector, vector + 1
.endr
.noaltmacro

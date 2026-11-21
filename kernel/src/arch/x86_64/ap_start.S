/* Copied verbatim to the reserved low page. All code references are relative;
 * the adjacent data page supplies physical far pointers and per-AP mailboxes.
 * Intel SDM325384-081US chapters9/10/11, INIT/SIPI and IA-32e activation.
 */
.section .rodata.ap_start,"a"
.balign 16
.global door_ap_start_begin
.global door_ap_start_end
.global door_ap_start_protected
.global door_ap_start_long
door_ap_start_begin:
.code16
    cli
    cld
    xor %eax, %eax
    mov %cs, %ax
    mov %ax, %ds
    mov %ax, %es
    shl $4, %eax
    mov %eax, %esi
    lgdtl 0x1010
    mov %cr0, %eax
    or $1, %eax
    mov %eax, %cr0
    ljmpl *0x1018
.code32
door_ap_start_protected:
    mov $16, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    lea 0x1080(%esi), %ebp
    mov $1, %eax
    cpuid
    shr $24, %ebx
    mov $4, %ecx
1:  cmp %ebx, (%ebp)
    je 2f
    add $64, %ebp
    loop 1b
3:  cli
    hlt
    jmp 3b
2:  mov 0x1000(%esi), %eax
    mov %eax, %cr3
    mov %cr4, %eax
    or $0x620, %eax
    mov %eax, %cr4
    mov $0xc0000080, %ecx
    rdmsr
    or $0x900, %eax
    wrmsr
    mov %cr0, %eax
    and $0xfffffff3, %eax
    or $0x80010023, %eax
    mov %eax, %cr0
    ljmpl *0x1020(%esi)
.code64
door_ap_start_long:
    mov 8(%rbp), %rsp
    and $-16, %rsp
    mov 4(%rbp), %edi
    lea 16(%rbp), %rsi
    /* RSI above now denotes IST data. Get the fixed common entry from the
     * adjacent page using the mailbox offset reconstructed from CPU slot.
     */
    mov %ebp, %eax
    and $-4096, %rax
    mov 8(%rax), %rax
    xor %ebp, %ebp
    call *%rax
4:  cli
    hlt
    jmp 4b
door_ap_start_end:
.code64

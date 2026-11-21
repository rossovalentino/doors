/* IRQs are disabled and the VM owner retains the user mapping for this call.
 * REP MOVSB is the only recoverable instruction. A fault resumes the common
 * cleanup with RCX's architecturally precise remaining byte count. */
.section .text,"ax"
.global door_user_copy
.type door_user_copy,@function
door_user_copy:
    movq %cr3, %r8
    movq %rcx, %cr3
    movq %rdx, %rcx
    cld
.global door_user_copy_instruction
door_user_copy_instruction:
    rep movsb
.global door_user_copy_fixup
door_user_copy_fixup:
    movq %r8, %cr3
    movq %rcx, %rax
    retq

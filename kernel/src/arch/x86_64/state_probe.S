/* Original context preservation fixture. The only progress source is a timer
 * callback's aligned per-thread counter; there is no cooperative yield here.
 */
.section .text,"ax"
.global door_state_probe
.type door_state_probe,@function
door_state_probe:
    push %rbx
    push %rbp
    push %r12
    push %r13
    push %r14
    push %r15
    sub $88, %rsp
    mov %rdi, 0(%rsp)
    mov %rsi, 8(%rsp)
    mov %rdx, 16(%rsp)
    movq $2000000, 24(%rsp)
    mov %rdi, %rax
    movq %rax, %xmm0
    punpcklqdq %xmm0, %xmm0
.irp reg,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
    movdqa %xmm0, %xmm\reg
.endr
.irp reg,0,1,2,3,4,5,6,7
    movq %rax, %mm\reg
.endr
.irp reg,rbx,rcx,rdx,rbp,rsi,rdi,r8,r9,r10,r11,r12,r13,r14,r15
    mov %rax, %\reg
.endr

.macro CHECK_CONTEXT
.irp reg,rax,rbx,rcx,rdx,rbp,rsi,rdi,r8,r9,r10,r11,r12,r13,r14,r15
    cmp 0(%rsp), %\reg
    jne door_state_bad_gpr
.endr
.irp reg,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
    movdqu %xmm\reg, 48(%rsp)
    cmp %rax, 48(%rsp)
    jne door_state_bad_sse
    cmp %rax, 56(%rsp)
    jne door_state_bad_sse
.endr
    cmp %rax, %fs:0
    jne door_state_bad_tls
.endm

1:  CHECK_CONTEXT
.irp reg,0,1,2,3,4,5,6,7
    movq %mm\reg, 32(%rsp)
    cmp %rax, 32(%rsp)
    jne door_state_bad_mmx
.endr
    mov 8(%rsp), %rax
    mov (%rax), %rax
    cmp 16(%rsp), %rax
    mov 0(%rsp), %rax
    jae 2f
    decq 24(%rsp)
    jz door_state_timeout
    pause
    jmp 1b
2:  emms
    fildq 0(%rsp)
    mov 8(%rsp), %rax
    mov (%rax), %rax
    add $2, %rax
    mov %rax, 16(%rsp)
    mov 0(%rsp), %rax
    movq $2000000, 24(%rsp)
3:  CHECK_CONTEXT
    mov 8(%rsp), %rax
    mov (%rax), %rax
    cmp 16(%rsp), %rax
    mov 0(%rsp), %rax
    jae 4f
    decq 24(%rsp)
    jz door_state_timeout
    pause
    jmp 3b
4:  fistpq 32(%rsp)
    cmp %rax, 32(%rsp)
    jne door_state_bad_x87
    xor %eax, %eax
    jmp door_state_done
door_state_bad_gpr:
    mov $1, %eax
    jmp door_state_done
door_state_bad_sse:
    mov $2, %eax
    jmp door_state_done
door_state_bad_mmx:
    mov $4, %eax
    jmp door_state_done
door_state_bad_x87:
    mov $8, %eax
    jmp door_state_done
door_state_bad_tls:
    mov $16, %eax
    jmp door_state_done
door_state_timeout:
    mov $32, %eax
door_state_done:
    emms
    add $88, %rsp
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    pop %rbx
    ret

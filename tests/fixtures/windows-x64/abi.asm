; Independent Win64 caller: preserve our caller's state, seed every nonvolatile
; GPR/XMM register, then call ordinary Windows APIs through the C callback.
option casemap:none

.const
ALIGN 16
patterns dq 0101010101010101h, 0202020202020202h
         dq 0303030303030303h, 0404040404040404h
         dq 0505050505050505h, 0606060606060606h
         dq 0707070707070707h, 0808080808080808h
         dq 0909090909090909h, 0a0a0a0a0a0a0a0ah
         dq 0b0b0b0b0b0b0b0bh, 0c0c0c0c0c0c0c0ch
         dq 0d0d0d0d0d0d0d0dh, 0e0e0e0e0e0e0e0eh
         dq 0f0f0f0f0f0f0f0fh, 1010101010101010h
         dq 1111111111111111h, 1212121212121212h
         dq 1313131313131313h, 1414141414141414h

.code
PUBLIC VerifyAbi
VerifyAbi PROC FRAME
    push rbp
    .pushreg rbp
    push rbx
    .pushreg rbx
    push rsi
    .pushreg rsi
    push rdi
    .pushreg rdi
    push r12
    .pushreg r12
    push r13
    .pushreg r13
    push r14
    .pushreg r14
    push r15
    .pushreg r15
    sub rsp, 216
    .allocstack 216
    movdqu xmmword ptr [rsp+48], xmm6
    .savexmm128 xmm6, 48
    movdqu xmmword ptr [rsp+64], xmm7
    .savexmm128 xmm7, 64
    movdqu xmmword ptr [rsp+80], xmm8
    .savexmm128 xmm8, 80
    movdqu xmmword ptr [rsp+96], xmm9
    .savexmm128 xmm9, 96
    movdqu xmmword ptr [rsp+112], xmm10
    .savexmm128 xmm10, 112
    movdqu xmmword ptr [rsp+128], xmm11
    .savexmm128 xmm11, 128
    movdqu xmmword ptr [rsp+144], xmm12
    .savexmm128 xmm12, 144
    movdqu xmmword ptr [rsp+160], xmm13
    .savexmm128 xmm13, 160
    movdqu xmmword ptr [rsp+176], xmm14
    .savexmm128 xmm14, 176
    movdqu xmmword ptr [rsp+192], xmm15
    .savexmm128 xmm15, 192
    .endprolog
    mov qword ptr [rsp+32], rcx
    mov qword ptr [rsp+40], rsp
    mov rbp, 16161616h
    mov rbx, 17171717h
    mov rsi, 18181818h
    mov rdi, 19191919h
    mov r12, 20202020h
    mov r13, 21212121h
    mov r14, 22222222h
    mov r15, 23232323h
    movdqu xmm6, xmmword ptr [patterns]
    movdqu xmm7, xmmword ptr [patterns+16]
    movdqu xmm8, xmmword ptr [patterns+32]
    movdqu xmm9, xmmword ptr [patterns+48]
    movdqu xmm10, xmmword ptr [patterns+64]
    movdqu xmm11, xmmword ptr [patterns+80]
    movdqu xmm12, xmmword ptr [patterns+96]
    movdqu xmm13, xmmword ptr [patterns+112]
    movdqu xmm14, xmmword ptr [patterns+128]
    movdqu xmm15, xmmword ptr [patterns+144]
    call entry_bridge
    cmp eax, 1
    jne restore_state
    cmp rsp, qword ptr [rsp+40]
    jne failed
    cmp rbp, 16161616h
    jne failed
    cmp rbx, 17171717h
    jne failed
    cmp rsi, 18181818h
    jne failed
    cmp rdi, 19191919h
    jne failed
    cmp r12, 20202020h
    jne failed
    cmp r13, 21212121h
    jne failed
    cmp r14, 22222222h
    jne failed
    cmp r15, 23232323h
    jne failed
check_xmm MACRO register, offset
    movdqa xmm0, register
    pcmpeqb xmm0, xmmword ptr [patterns+offset]
    pmovmskb eax, xmm0
    cmp eax, 0ffffh
    jne failed
ENDM
    check_xmm xmm6, 0
    check_xmm xmm7, 16
    check_xmm xmm8, 32
    check_xmm xmm9, 48
    check_xmm xmm10, 64
    check_xmm xmm11, 80
    check_xmm xmm12, 96
    check_xmm xmm13, 112
    check_xmm xmm14, 128
    check_xmm xmm15, 144
    mov eax, 1
    jmp restore_state
failed:
    mov eax, 201
restore_state:
    movdqu xmm6, xmmword ptr [rsp+48]
    movdqu xmm7, xmmword ptr [rsp+64]
    movdqu xmm8, xmmword ptr [rsp+80]
    movdqu xmm9, xmmword ptr [rsp+96]
    movdqu xmm10, xmmword ptr [rsp+112]
    movdqu xmm11, xmmword ptr [rsp+128]
    movdqu xmm12, xmmword ptr [rsp+144]
    movdqu xmm13, xmmword ptr [rsp+160]
    movdqu xmm14, xmmword ptr [rsp+176]
    movdqu xmm15, xmmword ptr [rsp+192]
    add rsp, 216
    pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbx
    pop rbp
    ret
VerifyAbi ENDP

; A normal Win64 callee has RSP mod16=8 and four writable home slots. Tail-call
; the C callback with exactly that entry state; no private calling convention.
entry_bridge PROC
    mov rax, rsp
    and eax, 15
    cmp eax, 8
    jne alignment_failed
    mov qword ptr [rsp+8], 1111h
    mov qword ptr [rsp+16], 2222h
    mov qword ptr [rsp+24], 3333h
    mov qword ptr [rsp+32], 4444h
    jmp rcx
alignment_failed:
    mov eax, 202
    ret
entry_bridge ENDP
END

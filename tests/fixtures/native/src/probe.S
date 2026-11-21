.section .text.entry,"ax"
.global door_user_entry
.type door_user_entry,@function
door_user_entry:
    mov rax, rsp
    and eax, 15
    cmp eax, 8
    jne probe_fail
    cmp rsi, 256
    jne probe_fail
    movabs rax, 0x31525355524f4f44
    cmp qword ptr [rdi], rax
    jne probe_fail
    cmp word ptr [rdi+8], 1
    jne probe_fail
    cmp dword ptr [rdi+12], 256
    jne probe_fail
    sub rsp, 256
    mov [rsp], rdi
    mov [rsp+8], rsp
    mov rax, [rdi+48]
    mov [rsp+16], rax
    mov rax, [rdi+16]
    mov [rsp+24], rax
    movdqu xmm0, [rip+probe_vector]
    movdqu xmm15, [rip+probe_vector]
    fld1
    movabs rbx, 0xb1b2b3b4b5b6b7b8
    movabs rbp, 0xc1c2c3c4c5c6c7c8
    movabs r12, 0x1213141516171819
    movabs r13, 0x232425262728292a
    movabs r14, 0x3435363738393a3b
    movabs r15, 0x45464748494a4b4c
    movabs rdi, 0x0123456789abcdef
    movabs rsi, 0xfedcba9876543210
    movabs rdx, 0x8000000000000001
    mov r10, -7
    movabs r8, 0x13579bdf2468ace0
    movabs r9, 0x0f1e2d3c4b5a6978
    movabs rax, 0xd001000000000700
    syscall
    test rax, rax
    jne probe_fail
    movabs rax, 0x0123456789abcdef
    cmp rdi, rax
    jne probe_fail
    movabs rax, 0xfedcba9876543210
    cmp rsi, rax
    jne probe_fail
    cmp r10, -7
    jne probe_fail
    movabs rax, 0x13579bdf2468ace0
    cmp r8, rax
    jne probe_fail
    movabs rax, 0x0f1e2d3c4b5a6978
    cmp r9, rax
    jne probe_fail
    movabs rax, 0xb1b2b3b4b5b6b7b8
    cmp rbx, rax
    jne probe_fail
    movabs rax, 0xc1c2c3c4c5c6c7c8
    cmp rbp, rax
    jne probe_fail
    movabs rax, 0x1213141516171819
    cmp r12, rax
    jne probe_fail
    movabs rax, 0x232425262728292a
    cmp r13, rax
    jne probe_fail
    movabs rax, 0x3435363738393a3b
    cmp r14, rax
    jne probe_fail
    movabs rax, 0x45464748494a4b4c
    cmp r15, rax
    jne probe_fail
    cmp [rsp+8], rsp
    jne probe_fail
    // Compute the documented checksum independently, using literal rotations.
    mov rax, rdi
    rol rax, 1
    mov rcx, rsi
    rol rcx, 7
    xor rax, rcx
    movabs rcx, 0x8000000000000001
    rol rcx, 13
    xor rax, rcx
    mov rcx, r10
    rol rcx, 23
    xor rax, rcx
    mov rcx, r8
    rol rcx, 37
    xor rax, rcx
    mov rcx, r9
    rol rcx, 53
    xor rax, rcx
    movabs rcx, 0xa5c36f19d2478be0
    xor rax, rcx
    cmp rax, rdx
    jne probe_fail
    mov rax, fs:[0]
    cmp rax, [rsp+16]
    jne probe_fail
    mov rax, fs:[8]
    cmp rax, [rsp+24]
    jne probe_fail
    movdqu [rsp+32], xmm0
    movdqu [rsp+48], xmm15
    mov rax, [rip+probe_vector]
    cmp [rsp+32], rax
    jne probe_fail
    cmp [rsp+48], rax
    jne probe_fail
    mov rax, [rip+probe_vector+8]
    cmp [rsp+40], rax
    jne probe_fail
    cmp [rsp+56], rax
    jne probe_fail
    fstp qword ptr [rsp+64]
    movabs rax, 0x3ff0000000000000
    cmp [rsp+64], rax
    jne probe_fail
    // Untagged and reserved namespace bits cannot select a native operation.
    mov eax, 0x400
    syscall
    cmp rax, 1
    jne probe_fail
    test rdx, rdx
    jne probe_fail
    movabs rax, 0xd001000100000400
    syscall
    cmp rax, 2
    jne probe_fail
    test rdx, rdx
    jne probe_fail
    movabs rax, 0xd0010000ffffff00
    syscall
    cmp rax, 1
    jne probe_fail
    test rdx, rdx
    jne probe_fail
    lea rdi, [rsp+80]
    movabs rax, 0xa5a5a5a5a5a5a5a5
    mov ecx, 8
    rep stosq
    lea rdi, [rsp+80]
    mov esi, 63
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    xor r9d, r9d
    movabs rax, 0xd001000000000000
    syscall
    cmp rax, 3
    jne probe_fail
    test rdx, rdx
    jne probe_fail
    lea rdi, [rsp+80]
    movabs rax, 0xa5a5a5a5a5a5a5a5
    mov ecx, 8
probe_unchanged:
    cmp [rdi], rax
    jne probe_fail
    add rdi, 8
    dec ecx
    jne probe_unchanged
    lea rdi, [rip+probe_pass]
    mov esi, 22
    movabs rax, 0xd001000000000002
    syscall
    test rax, rax
    jne probe_fail
    mov edi, 55
    xor esi, esi
    xor edx, edx
    movabs rax, 0xd001000000000001
    syscall
    ud2
probe_fail:
    mov rdi, -55
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    xor r9d, r9d
    movabs rax, 0xd001000000000001
    syscall
    ud2
.size door_user_entry,.-door_user_entry
.section .rodata,"a"
.balign 16
probe_vector:
    .quad 0x1020304050607080, 0x90a0b0c0d0e0f001
probe_pass:
    .ascii "DOOR_P05 RAW_ABI PASS\n"

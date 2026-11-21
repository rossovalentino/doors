EXTERN probe_resume:QWORD
.code
ProbeReadOnly PROC FRAME
    sub rsp, 28h
    .allocstack 28h
    .endprolog
    lea rax, resume_probe
    mov probe_resume, rax
    pcmpeqd xmm0, xmm0
    mov BYTE PTR [rcx], 99h
resume_probe:
    pmovmskb eax, xmm0
    cmp eax, 0ffffh
    sete al
    movzx eax, al
    add rsp, 28h
    ret
ProbeReadOnly ENDP
ClobberSimd PROC
    pxor xmm0, xmm0
    ret
ClobberSimd ENDP
END

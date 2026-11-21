.section .text.boot,"ax"
.global door_entry
.type door_entry,@function
door_entry:
    cli
    cld
    xor %rbp, %rbp
    call kernel_main
1:  cli
    hlt
    jmp 1b

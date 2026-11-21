; PE32 loader fixture (P17): one SEH handler registered with .SAFESEH, so
; the linker emits a SafeSEH table (__safe_se_handler_table) naming it.
.386
.model flat, stdcall
.code
door_handler proc C
    mov eax, 1 ; ExceptionContinueSearch
    ret
door_handler endp
.SAFESEH door_handler
end

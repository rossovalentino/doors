#include <windows.h>

static DWORD attached;
static DWORD tls_index = TLS_OUT_OF_INDEXES;
static volatile LONG active_threads;
static volatile LONG thread_errors;
__declspec(thread) static DWORD compiled_value = 0x11223344;
static DWORD compiler_anchor = 0xabcdef12;
__declspec(thread) static DWORD *compiled_pointer = &compiler_anchor;
__declspec(dllexport) DWORD WINAPI CompilerValue(void) { return compiled_pointer == &compiler_anchor && *compiled_pointer == 0xabcdef12 ? compiled_value : 0; }
void NTAPI CompilerTlsCallback(PVOID module, DWORD reason, PVOID reserved) {
    (void)module;
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (compiled_pointer != &compiler_anchor || *compiled_pointer != 0xabcdef12) InterlockedIncrement(&thread_errors);
        if (compiled_value != 0x11223344) InterlockedIncrement(&thread_errors);
        compiled_value = reason == DLL_PROCESS_ATTACH ? 0x23456789 : 0x3456789a;
    } else if (reason == DLL_THREAD_DETACH) {
        compiled_value = compiled_value == 0x3456789a ? 0x456789ab : 0x56789abc;
    } else if (reason == DLL_PROCESS_DETACH) {
        compiled_value = 0x6789abcd;
    }
}
__declspec(dllexport) DWORD WINAPI LifecycleState(void) {
    return attached == 1 && !active_threads && !thread_errors && compiled_value == 0x23456789 && TlsGetValue(tls_index) == (LPVOID)0x12345678 ? 1 : 0;
}

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID reserved) {
    DWORD count = 0;
    const char *text;
    DWORD bytes;
    if (!module) return FALSE;
    if (reason == DLL_PROCESS_ATTACH) {
        typedef BOOL (WINAPI *ALLOW_ATTACH)(void);
        ALLOW_ATTACH allow = (ALLOW_ATTACH)(ULONG_PTR)GetProcAddress(GetModuleHandleW(NULL), "AllowDllAttach");
        if (!allow) { WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),"DLL_REJECT_EXPORT\r\n",19,&count,NULL); return FALSE; }
        if (!allow()) return FALSE;
        if (reserved || attached || compiled_value != 0x23456789) {
            char diagnostic[]="DLL_REJECT_TLS=00000000\r\n";
            DWORD index;
            for(index=0;index<8;++index) diagnostic[15+index]="0123456789abcdef"[(compiled_value >> ((7-index)*4)) & 15];
            WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),diagnostic,sizeof(diagnostic)-1,&count,NULL);
            return FALSE;
        }
        tls_index = TlsAlloc();
        if (tls_index == TLS_OUT_OF_INDEXES || !TlsSetValue(tls_index, (LPVOID)0x12345678)) return FALSE;
        attached = 1;
        text = "WINDOWS_BASE_DLL attach=1\r\n";
        bytes = 27;
    } else if (reason == DLL_PROCESS_DETACH) {
        if (attached != 1) return FALSE;
        if (compiled_value != 0x6789abcd) return FALSE;
        if (active_threads != (reserved ? 1 : 0) || thread_errors) return FALSE;
        if (TlsGetValue(tls_index) != (LPVOID)0x12345678 || !TlsFree(tls_index)) return FALSE;
        attached = 0;
        text = reserved ? "WINDOWS_BASE_DLL detach=1 quiesced=1\r\n" : "WINDOWS_BASE_DLL detach=1 unloaded=1\r\n";
        bytes = 38;
    } else if (reason == DLL_THREAD_ATTACH) {
        if (compiled_value != 0x3456789a) InterlockedIncrement(&thread_errors);
        if (TlsGetValue(tls_index) != NULL || !TlsSetValue(tls_index, (LPVOID)0x5678)) InterlockedIncrement(&thread_errors);
        InterlockedIncrement(&active_threads);
        return TRUE;
    } else if (reason == DLL_THREAD_DETACH) {
        /* Existing threads receive detach after a late LoadLibrary without an
         * earlier THREAD_ATTACH notification. Their dynamic slot remains zero. */
        if (TlsGetValue(tls_index) == NULL) return compiled_value == 0x56789abc;
        if (compiled_value != 0x456789ab) InterlockedIncrement(&thread_errors);
        if (TlsGetValue(tls_index) != (LPVOID)0x5678) InterlockedIncrement(&thread_errors);
        InterlockedDecrement(&active_threads);
        return TRUE;
    } else return TRUE;
    return WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), text, bytes, &count, NULL) && count == bytes;
}

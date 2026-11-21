/* Original independent SDK TLS support for the fixture's no-CRT entry point. */
#include <windows.h>
#pragma section(".tls", long, read, write)
#pragma section(".tls$ZZZ", long, read, write)
#pragma section(".CRT$XLA", long, read)
#pragma section(".CRT$XLC", long, read)
#pragma section(".CRT$XLZ", long, read)
__declspec(allocate(".tls")) char _tls_start = 0;
__declspec(allocate(".tls$ZZZ")) char _tls_end = 0;
DWORD _tls_index = 0;
extern void NTAPI CompilerTlsCallback(PVOID module, DWORD reason, PVOID reserved);
__declspec(allocate(".CRT$XLA")) const PIMAGE_TLS_CALLBACK __xl_a = NULL;
__declspec(allocate(".CRT$XLC")) const PIMAGE_TLS_CALLBACK __xl_c = CompilerTlsCallback;
__declspec(allocate(".CRT$XLZ")) const PIMAGE_TLS_CALLBACK __xl_z = NULL;
const IMAGE_TLS_DIRECTORY64 _tls_used = {
    (ULONGLONG)&_tls_start, (ULONGLONG)&_tls_end,
    (ULONGLONG)&_tls_index, (ULONGLONG)(&__xl_a + 1), 0, 0
};

/* PE32 loader fixture (P17): an x86 console program, no C runtime, built
 * with the pinned MSVC x86 tools (see build.md). It imports x86dll.dll by
 * name and by ordinal, holds absolute addresses (HIGHLOW relocations), a
 * TLS directory with one callback, a load configuration with a /GS cookie
 * slot and a SafeSEH table naming one registered handler (handler.asm). */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

__declspec(dllimport) int __cdecl DoorAdd(int a, int b);
__declspec(dllimport) int __cdecl DoorTwice(int a);

/* TLS: a template of two words, an index, one callback. */
#pragma data_seg(".tls")
int tls_start = 0x1234;
#pragma data_seg(".tls$ZZZ")
int tls_end = 0x5678;
#pragma data_seg()
ULONG _tls_index;
static int callback_calls;
static void NTAPI tls_callback(PVOID module, DWORD reason, PVOID reserved) {
    (void)module;
    (void)reserved;
    callback_calls += (int)reason;
}
#pragma data_seg(".rdata$T")
PIMAGE_TLS_CALLBACK tls_callbacks[2] = {tls_callback, 0};
#pragma data_seg()
const IMAGE_TLS_DIRECTORY32 _tls_used = {
    (DWORD)&tls_start, (DWORD)&tls_end, (DWORD)&_tls_index, (DWORD)tls_callbacks, 0, 0};

/* The /GS cookie (its compiler default) and the SafeSEH table the linker
 * builds from handler.asm's registered handler. */
UINT_PTR __security_cookie = 0xbb40e64e;
extern PVOID __safe_se_handler_table[];
extern BYTE __safe_se_handler_count;
const IMAGE_LOAD_CONFIG_DIRECTORY32 _load_config_used = {
    sizeof(IMAGE_LOAD_CONFIG_DIRECTORY32),
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    (DWORD)&__security_cookie,
    (DWORD)__safe_se_handler_table,
    (DWORD)&__safe_se_handler_count,
};

/* Absolute addresses: a table of pointers the loader must relocate. */
static const char *const words[] = {"Doors", "x86", "loader"};
static int __stdcall add(int a, int b) { return DoorAdd(a, b); }
int (__stdcall *const functions[])(int, int) = {add};

/* Imports only x86dll.dll, so the bundle is these two images alone. */
int entry(void) {
    return functions[0](DoorTwice(20), 2) + (int)(words[1][0] != 'x') + callback_calls;
}

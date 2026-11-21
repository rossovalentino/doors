/* X86-A helper DLL (P17 M9), loaded by x86probe.exe from its own folder.
 * Linked at the executable's own base (0x400000), so the loader must
 * relocate it (IMAGE_REL_BASED_HIGHLOW fixups of the absolute pointers
 * below), with a DllMain that records what it saw. No C runtime, no
 * imports. (A TLS callback in a DLL like this one is not run by Windows 11's
 * WOW64 loader on LoadLibrary, as measured on the owner's PC, so the
 * fixture has none.) */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

static LONG main_reason, main_count;
/* Absolute addresses the linker writes and the loader must fix. */
static const char text[] = "relocated";
static const char *const table[2] = {"first", text};

BOOL WINAPI entry(HINSTANCE module, DWORD reason, LPVOID reserved) {
    (void)module;
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) {
        main_reason = (LONG)reason;
        main_count++;
    }
    return TRUE;
}

static int same(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
/* What the DLL saw: DllMain's reason and count, and the relocated
 * pointers' targets. */
__declspec(dllexport) void __cdecl x86dll_report(LONG *out) {
    out[0] = main_reason;
    out[1] = main_count;
    out[2] = same(table[1], "relocated") && same(table[0], "first");
    out[3] = (LONG)((DWORD)table[1] == (DWORD)text);
}

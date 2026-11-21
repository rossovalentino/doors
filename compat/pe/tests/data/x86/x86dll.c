/* PE32 loader fixture (P17): the x86 DLL x86app.exe imports (see build.md).
 * DoorAdd is exported by name, DoorTwice by ordinal only (x86dll.def);
 * both are __cdecl, so the import library names them undecorated. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

static int calls;
static int *const counter = &calls;

int __cdecl DoorAdd(int a, int b) {
    ++*counter;
    return a + b;
}
int __cdecl DoorTwice(int a) {
    ++*counter;
    return a * 2;
}
BOOL __stdcall DllMain(HINSTANCE module, DWORD reason, LPVOID reserved) {
    (void)module;
    (void)reason;
    (void)reserved;
    return TRUE;
}

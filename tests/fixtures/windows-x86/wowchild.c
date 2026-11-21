/* X86-C helper (P17 M9): the x64 program wowprobe.exe (32-bit) starts.
 *
 *   wowchild.exe report | verify
 *
 * report (read only): its exit code holds what it sees of itself, one bit
 * each: 1 ProgramFiles is C:\Program Files, 2 PROCESSOR_ARCHITECTURE is
 * AMD64, 4 PROCESSOR_ARCHITEW6432 is not set, 8 IsWow64Process says no,
 * 16 CommonProgramFiles is C:\Program Files\Common Files.
 *
 * verify (guest only, elevated): checks through the 64-bit views what the
 * 32-bit installer wrote and removes it: 1 HKLM\SOFTWARE\WOW6432Node\
 * DoorsWow64Test holds Seven = 7, 2 HKLM\SOFTWARE\DoorsWow64Test does not
 * exist, 4 KEY_WOW64_32KEY opens SOFTWARE\DoorsWow64Test, 8 the file is in
 * C:\Windows\SysWOW64, 16 not in System32, 32 C:\Program Files (x86)\
 * DoorsWow64Test\app.txt exists. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>

static int wsame(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static int is(const wchar_t *name, const wchar_t *expected) {
    wchar_t value[260];
    DWORD n = GetEnvironmentVariableW(name, value, 260);
    return n > 0 && n < 260 && wsame(value, expected);
}
static int last_word_is(const wchar_t *line, const wchar_t *word) {
    const wchar_t *w = line, *p;
    for (p = line; *p; p++)
        if (*p == L' ' && p[1] && p[1] != L' ') w = p + 1;
    while (*w && *w == *word) w++, word++;
    return *w == *word;
}
static DWORD report(void) {
    DWORD bits = 0;
    BOOL wow = 2;
    wchar_t value[16];
    if (is(L"ProgramFiles", L"C:\\Program Files")) bits |= 1;
    if (is(L"PROCESSOR_ARCHITECTURE", L"AMD64")) bits |= 2;
    if (GetEnvironmentVariableW(L"PROCESSOR_ARCHITEW6432", value, 16) == 0) bits |= 4;
    if (IsWow64Process(GetCurrentProcess(), &wow) && !wow) bits |= 8;
    if (is(L"CommonProgramFiles", L"C:\\Program Files\\Common Files")) bits |= 16;
    return bits;
}
static DWORD verify(void) {
    DWORD bits = 0, seven = 0, bytes = 4;
    HKEY key;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\WOW6432Node\\DoorsWow64Test", 0, KEY_READ, &key) == ERROR_SUCCESS) {
        if (RegQueryValueExW(key, L"Seven", NULL, NULL, (BYTE *)&seven, &bytes) == ERROR_SUCCESS && seven == 7) bits |= 1;
        RegCloseKey(key);
    }
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\DoorsWow64Test", 0, KEY_READ, &key) == ERROR_FILE_NOT_FOUND) bits |= 2;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\DoorsWow64Test", 0, KEY_READ | KEY_WOW64_32KEY, &key) == ERROR_SUCCESS) {
        bits |= 4;
        RegCloseKey(key);
    }
    if (GetFileAttributesW(L"C:\\Windows\\SysWOW64\\doorswow64.txt") != INVALID_FILE_ATTRIBUTES) bits |= 8;
    if (GetFileAttributesW(L"C:\\Windows\\System32\\doorswow64.txt") == INVALID_FILE_ATTRIBUTES) bits |= 16;
    if (GetFileAttributesW(L"C:\\Program Files (x86)\\DoorsWow64Test\\app.txt") != INVALID_FILE_ATTRIBUTES) bits |= 32;
    /* App Paths is shared: what the 32-bit installer wrote there is the
     * same key in the 64-bit view, and not under WOW6432Node. */
    seven = 0;
    bytes = 4;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\doorswow64.exe", 0, KEY_READ, &key) == ERROR_SUCCESS) {
        if (RegQueryValueExW(key, L"Seven", NULL, NULL, (BYTE *)&seven, &bytes) == ERROR_SUCCESS && seven == 7) bits |= 64;
        RegCloseKey(key);
    }
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\doorswow64.exe", 0, KEY_READ, &key) == ERROR_SUCCESS) {
        bits &= ~64u;
        RegCloseKey(key);
    }
    RegDeleteKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\doorswow64.exe", KEY_WOW64_64KEY, 0);
    RegDeleteKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\DoorsWow64Test", KEY_WOW64_32KEY, 0);
    DeleteFileW(L"C:\\Windows\\SysWOW64\\doorswow64.txt");
    DeleteFileW(L"C:\\Program Files (x86)\\DoorsWow64Test\\app.txt");
    RemoveDirectoryW(L"C:\\Program Files (x86)\\DoorsWow64Test");
    return bits;
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    ExitProcess(last_word_is(line, L"verify") ? verify() : report());
}

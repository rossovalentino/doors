/* X86-C probe (P17 M9): what a 32-bit program sees of WOW64's views,
 * MSVC x86, no C runtime:
 *
 *   wowprobe.exe raw | check | guest
 *
 * raw/check read only (safe on any PC): the environment WOW64 gives a
 * 32-bit process, SHGetKnownFolderPath's Program Files and system folders,
 * the registry's 32-bit and 64-bit views of HKLM\SOFTWARE\Microsoft\Windows
 * \CurrentVersion (ProgramFilesDir), WOW6432Node and a shared key, the
 * file-system redirection switches, and an x64 child (wowchild.exe): what
 * IsWow64Process(2) say of it and what it sees of its own environment
 * (reported as its exit code). raw prints the lines (wowprobe.expected.h
 * is generated from Windows 11's output); check compares.
 *
 * guest additionally writes, as an elevated 32-bit installer would: a
 * HKLM\SOFTWARE key (into WOW6432Node), a file into C:\Windows\System32
 * (into SysWOW64) and one into %ProgramFiles% (Program Files (x86)), then
 * runs `wowchild.exe verify`, which checks them through the 64-bit views
 * and removes them. Never run `guest` on a real Windows PC. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <shlobj.h>
#include <knownfolders.h>

#pragma function(memset, memcpy)
void *memset(void *d, int v, size_t n) {
    unsigned char *p = (unsigned char *)d;
    while (n--) *p++ = (unsigned char)v;
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = (unsigned char *)d;
    const unsigned char *q = (const unsigned char *)s;
    while (n--) *p++ = *q++;
    return d;
}

static HANDLE out;
static char captured[8192];
static int captured_length, capturing;
static void put(const char *s) {
    DWORD w;
    int n = 0;
    while (s[n]) n++;
    if (capturing) {
        int k;
        for (k = 0; k < n && captured_length < (int)sizeof captured - 1; k++) captured[captured_length++] = s[k];
        captured[captured_length] = 0;
        return;
    }
    WriteFile(out, s, (DWORD)n, &w, NULL);
}
static void num(long v) {
    char b[16];
    int i = 15, neg = v < 0;
    unsigned long u = neg ? (unsigned long)-v : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (neg) b[--i] = '-';
    put(b + i);
}
static void hex(unsigned long v) {
    char b[11];
    int i;
    b[0] = '0';
    b[1] = 'x';
    for (i = 0; i < 8; i++) b[2 + i] = "0123456789abcdef"[(v >> (28 - 4 * i)) & 15];
    b[10] = 0;
    put(b);
}
static void nl(void) { put("\r\n"); }
static void wput(const wchar_t *s) {
    char b[2];
    b[1] = 0;
    while (*s) {
        b[0] = *s < 128 ? (char)*s : '?';
        put(b);
        s++;
    }
}
static void wlower(const wchar_t *s) {
    char b[2];
    b[1] = 0;
    while (*s) {
        wchar_t c = *s >= L'A' && *s <= L'Z' ? (wchar_t)(*s + 32) : *s;
        b[0] = c < 128 ? (char)c : '?';
        put(b);
        s++;
    }
}
static void wcopy(wchar_t *d, const wchar_t *s) {
    while ((*d++ = *s++) != 0) {
    }
}
static void wcat(wchar_t *d, const wchar_t *s) {
    while (*d) d++;
    wcopy(d, s);
}

static void variable(const wchar_t *name) {
    wchar_t value[260];
    DWORD n = GetEnvironmentVariableW(name, value, 260);
    wput(name);
    put("=");
    if (n == 0 || n >= 260)
        put("(none)");
    else
        wput(value);
    nl();
}
static void environment_step(void) {
    variable(L"PROCESSOR_ARCHITECTURE");
    variable(L"PROCESSOR_ARCHITEW6432");
    variable(L"ProgramFiles");
    variable(L"ProgramFiles(x86)");
    variable(L"ProgramW6432");
    variable(L"CommonProgramFiles");
    variable(L"CommonProgramFiles(x86)");
    variable(L"CommonProgramW6432");
}

static void folder(const char *name, REFKNOWNFOLDERID id) {
    PWSTR path = NULL;
    HRESULT hr = SHGetKnownFolderPath(id, 0, NULL, &path);
    put(name);
    put("=");
    if (hr == S_OK && path)
        wlower(path); /* the Windows folder's case varies by install */
    else
        hex((unsigned long)hr);
    nl();
    /* The path is task memory; this probe imports no ole32 to free it. */
}
static void folders_step(void) {
    folder("ProgramFiles", &FOLDERID_ProgramFiles);
    folder("ProgramFilesX86", &FOLDERID_ProgramFilesX86);
    folder("ProgramFilesX64", &FOLDERID_ProgramFilesX64);
    folder("ProgramFilesCommon", &FOLDERID_ProgramFilesCommon);
    folder("ProgramFilesCommonX64", &FOLDERID_ProgramFilesCommonX64);
    folder("System", &FOLDERID_System);
    folder("SystemX86", &FOLDERID_SystemX86);
    folder("Windows", &FOLDERID_Windows);
}

static void value(const char *label, REGSAM view, const wchar_t *name) {
    HKEY key;
    wchar_t data[260];
    DWORD bytes = sizeof data, type = 0;
    LONG r = RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion", 0, KEY_READ | view, &key);
    put(label);
    put(" ");
    wput(name);
    put("=");
    if (r != ERROR_SUCCESS) {
        put("open ");
        num(r);
        nl();
        return;
    }
    r = RegQueryValueExW(key, name, NULL, &type, (BYTE *)data, &bytes);
    if (r == ERROR_SUCCESS && type == REG_SZ)
        wput(data);
    else {
        put("query ");
        num(r);
    }
    nl();
    RegCloseKey(key);
}
static void opens(const char *label, const wchar_t *path, REGSAM view) {
    HKEY key;
    LONG r = RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, 0, KEY_READ | view, &key);
    put(label);
    put("=");
    num(r);
    nl();
    if (r == ERROR_SUCCESS) RegCloseKey(key);
}
static void registry_step(void) {
    value("default", 0, L"ProgramFilesDir");
    value("64", KEY_WOW64_64KEY, L"ProgramFilesDir");
    value("32", KEY_WOW64_32KEY, L"ProgramFilesDir");
    value("default", 0, L"CommonFilesDir");
    value("64", KEY_WOW64_64KEY, L"CommonFilesDir");
    value("default", 0, L"ProgramW6432Dir");
    opens("node-64", L"SOFTWARE\\WOW6432Node", KEY_WOW64_64KEY);
    opens("app-paths", L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths", 0);
    opens("app-paths-64", L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths", KEY_WOW64_64KEY);
}

static void redirection_step(void) {
    PVOID old = (PVOID)1;
    BOOL off = Wow64DisableWow64FsRedirection(&old);
    BOOL back = Wow64RevertWow64FsRedirection(old);
    DWORD sysnative = GetFileAttributesW(L"C:\\Windows\\Sysnative");
    wchar_t wow[MAX_PATH];
    UINT n = GetSystemWow64DirectoryW(wow, MAX_PATH);
    DWORD wow_attributes = n ? GetFileAttributesW(wow) : INVALID_FILE_ATTRIBUTES;
    put("redirection disable=");
    num(off);
    put(" old=");
    num(old == NULL);
    put(" revert=");
    num(back);
    put(" sysnative=");
    num(sysnative != INVALID_FILE_ATTRIBUTES && (sysnative & FILE_ATTRIBUTE_DIRECTORY) != 0);
    put(" syswow64=");
    num(wow_attributes != INVALID_FILE_ATTRIBUTES && (wow_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0);
    nl();
}

/* The x64 child next to this program, run with `arguments`; its exit code. */
static DWORD child(const wchar_t *arguments, BOOL *wow, USHORT *machine, USHORT *native) {
    wchar_t path[MAX_PATH + 40], line[MAX_PATH + 80];
    DWORD n = GetModuleFileNameW(NULL, path, MAX_PATH), code = 0xffffffff;
    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    typedef BOOL(WINAPI * Wow2)(HANDLE, USHORT *, USHORT *);
    Wow2 f = (Wow2)GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "IsWow64Process2");
    while (n && path[n - 1] != L'\\' && path[n - 1] != L'/') n--;
    path[n] = 0;
    wcat(path, L"wowchild.exe");
    line[0] = L'"';
    line[1] = 0;
    wcat(line, path);
    wcat(line, L"\" ");
    wcat(line, arguments);
    memset(&si, 0, sizeof si);
    si.cb = sizeof si;
    if (!CreateProcessW(path, line, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)) return 0xfffffffe;
    *wow = 2;
    IsWow64Process(pi.hProcess, wow);
    *machine = 0xffff;
    *native = 0xffff;
    if (f) f(pi.hProcess, machine, native);
    WaitForSingleObject(pi.hProcess, 20000);
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return code;
}
static void child_step(void) {
    BOOL wow = 0, self = 2;
    USHORT machine = 0, native = 0;
    DWORD code = child(L"report", &wow, &machine, &native);
    IsWow64Process(GetCurrentProcess(), &self);
    put("child exit=");
    num((long)code);
    put(" wow64=");
    num(wow);
    put(" machine=");
    hex(machine);
    put(" native=");
    hex(native);
    put(" self=");
    num(self);
    nl();
}

/* guest only: the elevated installer's writes, checked by the x64 child. */
static int installer(void) {
    HKEY key;
    DWORD seven = 7, w;
    wchar_t path[MAX_PATH + 40];
    HANDLE f;
    BOOL wow;
    USHORT machine, native;
    DWORD code;
    LONG r = RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\DoorsWow64Test", 0, NULL, 0, KEY_ALL_ACCESS, NULL, &key, NULL);
    int ok = r == ERROR_SUCCESS;
    if (ok) {
        ok = RegSetValueExW(key, L"Seven", 0, REG_DWORD, (const BYTE *)&seven, 4) == ERROR_SUCCESS;
        RegCloseKey(key);
    }
    /* A shared key (App Paths), written through the 32-bit default view. */
    r = RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\doorswow64.exe", 0, NULL, 0, KEY_ALL_ACCESS, NULL, &key, NULL);
    if (r == ERROR_SUCCESS) {
        if (RegSetValueExW(key, L"Seven", 0, REG_DWORD, (const BYTE *)&seven, 4) != ERROR_SUCCESS) ok = 0;
        RegCloseKey(key);
    } else {
        ok = 0;
    }
    put("WOW-C install registry=");
    num(ok);
    f = CreateFileW(L"C:\\Windows\\System32\\doorswow64.txt", GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    put(" system32=");
    num(f != INVALID_HANDLE_VALUE && WriteFile(f, "x86", 3, &w, NULL));
    if (f != INVALID_HANDLE_VALUE) CloseHandle(f);
    put(" sysnative=");
    num(GetFileAttributesW(L"C:\\Windows\\Sysnative\\doorswow64.txt") == INVALID_FILE_ATTRIBUTES);
    put(" syswow64=");
    num(GetFileAttributesW(L"C:\\Windows\\SysWOW64\\doorswow64.txt") != INVALID_FILE_ATTRIBUTES);
    GetEnvironmentVariableW(L"ProgramFiles", path, MAX_PATH);
    wcat(path, L"\\DoorsWow64Test");
    CreateDirectoryW(path, NULL);
    wcat(path, L"\\app.txt");
    f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    put(" programfiles=");
    num(f != INVALID_HANDLE_VALUE && WriteFile(f, "x86", 3, &w, NULL));
    if (f != INVALID_HANDLE_VALUE) CloseHandle(f);
    nl();
    code = child(L"verify", &wow, &machine, &native);
    put("WOW-C verify exit=");
    num((long)code);
    nl();
    /* A 32-bit setup program starts the installer by name: the search goes
     * through System32, which is SysWOW64 for it. Removing a product that
     * is not installed, silently, ends with ERROR_UNKNOWN_PRODUCT. */
    {
        wchar_t command[] = L"msiexec /x {D0025A00-0000-4000-8000-000000000000} /qn";
        STARTUPINFOW si;
        PROCESS_INFORMATION pi;
        DWORD installer_code = 0xffffffff;
        memset(&si, 0, sizeof si);
        si.cb = sizeof si;
        if (CreateProcessW(NULL, command, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)) {
            WaitForSingleObject(pi.hProcess, 30000);
            GetExitCodeProcess(pi.hProcess, &installer_code);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        } else {
            installer_code = 0x80000000 | GetLastError();
        }
        put("WOW-C msiexec exit=");
        num((long)installer_code);
        nl();
        if (installer_code != 1605) ok = 0;
    }
    return ok && code == 0x7f ? 0 : 1;
}

struct step {
    const char *name;
    void (*run)(void);
};
static const struct step steps[] = {
    {"environment", environment_step}, {"folders", folders_step}, {"registry", registry_step},
    {"redirection", redirection_step}, {"child", child_step},
};
static const char *const expected[] = {
#include "wowprobe.expected.h"
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[400];
    int n = line_length(s), k;
    if (n > 399) n = 399;
    for (k = 0; k < n; k++) b[k] = s[k];
    b[n] = 0;
    put(label);
    put(": ");
    put(b);
    put("\r\n");
}
static int compare(const char *got, const char *want, int *count, const char **g, const char **w) {
    int line = 1;
    *count = 0;
    for (;;) {
        int a = line_length(got), b = line_length(want), k;
        if (!*got && !*want) return 0;
        if (a != b) goto differ;
        for (k = 0; k < a; k++)
            if (got[k] != want[k]) goto differ;
        got += a;
        want += b;
        while (*got == '\r' || *got == '\n') got++;
        while (*want == '\r' || *want == '\n') want++;
        (*count)++;
        line++;
        continue;
    differ:
        *g = got;
        *w = want;
        return line;
    }
}
static int last_word_is(const wchar_t *line, const wchar_t *word) {
    const wchar_t *w = line, *p;
    for (p = line; *p; p++)
        if (*p == L' ' && p[1] && p[1] != L' ') w = p + 1;
    while (*w && *w == *word) w++, word++;
    return *w == *word;
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    int raw = last_word_is(line, L"raw"), guest = last_word_is(line, L"guest"), failed = 0, i;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        if (raw) {
            put("== ");
            put(steps[i].name);
            nl();
            steps[i].run();
            continue;
        }
        {
            int count, at;
            const char *g = 0, *w = 0;
            captured_length = 0;
            captured[0] = 0;
            capturing = 1;
            steps[i].run();
            capturing = 0;
            at = compare(captured, expected[i], &count, &g, &w);
            put("WOW-C ");
            num(i + 1);
            put(" ");
            put(steps[i].name);
            if (at == 0) {
                put(" ok ");
                num(count);
                nl();
            } else {
                put(" FAILED line ");
                num(at);
                nl();
                put_line("got", g);
                put_line("expected", w);
                failed = 1;
            }
        }
    }
    if (raw) ExitProcess(0);
    if (guest && installer()) failed = 1;
    put(failed ? "WOW-C FAILED\r\n" : "WOW-C PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

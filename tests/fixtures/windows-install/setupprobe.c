/* INSTALL-A: an EXE installer as NSIS and Inno Setup make them
 * (setupprobe.exe; its manifest asks for administrator rights, so the
 * shell asks User Account Control first). Windows SDK, no C runtime.
 *
 *   setupprobe.exe              installs Doors Setup Sample: C:\Program
 *                               Files\Doors Setup Sample with readme.txt
 *                               and uninstall.exe (a copy of itself), and
 *                               its Add/Remove Programs entry
 *   uninstall.exe               (the copy) runs itself from %TEMP% with
 *                               _?=<folder>, as NSIS uninstallers do
 *   ... _?=<folder>             removes the files, the folder and the
 *                               entry */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    unsigned char *d = destination;
    while (count--) *d++ = (unsigned char)value;
    return destination;
}

#define FOLDER L"C:\\Program Files\\Doors Setup Sample"
#define KEY L"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\DoorsSetupSample"

static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static void copy(wchar_t *d, const wchar_t *s) {
    while ((*d++ = *s++) != 0) {
    }
}
static void cat(wchar_t *d, const wchar_t *s) {
    copy(d + wlen(d), s);
}
static int ends_with_ci(const wchar_t *a, const wchar_t *tail) {
    int n = wlen(a), m = wlen(tail);
    if (n < m) return 0;
    for (int i = 0; i < m; i++) {
        wchar_t x = a[n - m + i], y = tail[i];
        if (x >= L'A' && x <= L'Z') x = (wchar_t)(x + 32);
        if (y >= L'A' && y <= L'Z') y = (wchar_t)(y + 32);
        if (x != y) return 0;
    }
    return 1;
}
static const wchar_t *find(const wchar_t *hay, const wchar_t *needle) {
    int m = wlen(needle);
    for (; *hay; hay++) {
        int i = 0;
        while (i < m && hay[i] == needle[i]) i++;
        if (i == m) return hay;
    }
    return NULL;
}
static LSTATUS set_string(HKEY key, const wchar_t *name, const wchar_t *value) {
    return RegSetValueExW(key, name, 0, REG_SZ, (const BYTE *)value, (DWORD)((wlen(value) + 1) * sizeof(wchar_t)));
}
static LSTATUS set_dword(HKEY key, const wchar_t *name, DWORD value) {
    return RegSetValueExW(key, name, 0, REG_DWORD, (const BYTE *)&value, sizeof value);
}

static UINT install(const wchar_t *self) {
    if (!CreateDirectoryW(FOLDER, NULL) && GetLastError() != ERROR_ALREADY_EXISTS) return 2;
    static wchar_t target[MAX_PATH];
    copy(target, FOLDER L"\\uninstall.exe");
    if (!CopyFileW(self, target, FALSE)) return 3;
    HANDLE readme = CreateFileW(FOLDER L"\\readme.txt", GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (readme == INVALID_HANDLE_VALUE) return 4;
    DWORD written = 0;
    WriteFile(readme, "Doors Setup Sample\r\n", 20, &written, NULL);
    CloseHandle(readme);
    HKEY key;
    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, KEY, 0, NULL, 0, KEY_ALL_ACCESS, NULL, &key, NULL) != ERROR_SUCCESS) return 5;
    static wchar_t uninstall[MAX_PATH + 4];
    copy(uninstall, L"\"");
    cat(uninstall, target);
    cat(uninstall, L"\"");
    SYSTEMTIME now;
    GetSystemTime(&now);
    wchar_t date[9];
    DWORD parts[3] = {now.wYear, now.wMonth, now.wDay};
    int widths[3] = {4, 2, 2};
    int at = 0;
    for (int p = 0; p < 3; p++) {
        for (int i = widths[p] - 1; i >= 0; i--) {
            DWORD v = parts[p];
            for (int k = 0; k < i; k++) v /= 10;
            date[at++] = (wchar_t)(L'0' + v % 10);
        }
    }
    date[8] = 0;
    LSTATUS r = set_string(key, L"DisplayName", L"Doors Setup Sample");
    r |= set_string(key, L"DisplayVersion", L"3.1.0");
    r |= set_string(key, L"Publisher", L"Macrohard Test");
    r |= set_string(key, L"InstallLocation", FOLDER);
    r |= set_string(key, L"UninstallString", uninstall);
    r |= set_string(key, L"DisplayIcon", target);
    r |= set_string(key, L"InstallDate", date);
    r |= set_dword(key, L"EstimatedSize", 12);
    r |= set_dword(key, L"NoModify", 1);
    r |= set_dword(key, L"NoRepair", 1);
    RegCloseKey(key);
    return r == ERROR_SUCCESS ? 0 : 6;
}

/* The uninstaller cannot delete itself while it runs: a copy in %TEMP%
 * does the work (NSIS's _?= convention). */
static UINT relaunch(const wchar_t *self) {
    static wchar_t temp[MAX_PATH], line[MAX_PATH * 2 + 16];
    DWORD n = GetTempPathW(MAX_PATH, temp);
    if (n == 0 || n > MAX_PATH - 16) return 7;
    cat(temp, L"Un_A.exe");
    if (!CopyFileW(self, temp, FALSE)) return 8;
    copy(line, L"\"");
    cat(line, temp);
    cat(line, L"\" _?=" FOLDER);
    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    for (unsigned i = 0; i < sizeof si; i++) ((char *)&si)[i] = 0;
    si.cb = sizeof si;
    if (!CreateProcessW(NULL, line, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)) return 9;
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 10;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return code;
}
static UINT uninstall(void) {
    DeleteFileW(FOLDER L"\\readme.txt");
    DeleteFileW(FOLDER L"\\uninstall.exe");
    RemoveDirectoryW(FOLDER);
    LSTATUS r = RegDeleteKeyW(HKEY_LOCAL_MACHINE, KEY);
    return r == ERROR_SUCCESS && GetFileAttributesW(FOLDER) == INVALID_FILE_ATTRIBUTES ? 0 : 11;
}

void entry(void) {
    static wchar_t self[MAX_PATH];
    GetModuleFileNameW(NULL, self, MAX_PATH);
    const wchar_t *line = GetCommandLineW();
    UINT code;
    if (find(line, L"_?=") != NULL) {
        code = uninstall();
    } else if (ends_with_ci(self, L"\\uninstall.exe")) {
        code = relaunch(self);
    } else {
        code = install(self);
    }
    ExitProcess(code);
}

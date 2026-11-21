/* SHELL-A probe (P16): an ordinary Windows console program (Windows SDK, no
 * C runtime) that uses shell32.dll and CreateProcess:
 *
 *   shellprobe.exe            the checks below
 *   shellprobe.exe child N    exits with code N (started by the checks)
 *   shellprobe.exe child env  13 when only its launcher's environment block is set
 *   shellprobe.exe child pipe answers "pong:" and its standard input on its
 *                             standard output (both pipes); exits 14
 *   shellprobe.exe tray       a notification icon; exits on its NIN_SELECT
 *                             (guest only)
 *   shellprobe.exe mark       copies itself to Downloads\downloaded.exe with
 *                             the mark of the web (Zone.Identifier, zone 3);
 *                             run under that name it exits 15 at once
 *   shellprobe.exe recycle    FOF_ALLOWUNDO deletes into the Recycle Bin
 *                             (guest only: it adds to the user's bin)
 *   shellprobe.exe openwith   ShellExecuteEx of a file no app is registered
 *                             for: ERROR_NO_ASSOCIATION without UI, else the
 *                             shell asks which app to use (guest only)
 *   shellprobe.exe openas     the "openas" verb on that file (guest only)
 *
 * Command-line splitting, known folders and their item ID lists,
 * SHCreateDirectoryEx, SHFileOperation (copy, rename, move, delete) in a
 * folder under %TEMP%, SHGetFileInfo for a file of an unregistered type
 * and a folder, ShellExecute(Ex) and CreateProcess of this program (the exit
 * code comes back), a command line longer than the startup record, icons
 * (the system error icon drawn, a file type's small icon, a stock icon,
 * the system image list) and SHBrowseForFolder driven by its callback.
 * Nothing outside its own temporary folder changes, so it
 * runs on Windows as reference.
 *
 * One "SHELL-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <shellapi.h>
#include <shlobj.h>
#include <knownfolders.h>
#include <commctrl.h>
#include <intrin.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
    return destination;
}

static const GUID folder_profile = {0x5E6C858F, 0x0E22, 0x4760, {0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73}};
static const GUID folder_local = {0xF1B32785, 0x6FBA, 0x4FCF, {0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91}};
static const GUID folder_unknown = {0xD00250D1, 0x0000, 0x0000, {0, 0, 0, 0, 0, 0, 0, 1}};

static HANDLE out;
static int length(const char *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static void put(const char *s) {
    DWORD written;
    WriteFile(out, s, (DWORD)length(s), &written, NULL);
}
static void number(long v) {
    char b[24];
    int i = 23;
    unsigned long u = v < 0 ? (unsigned long)(-v) : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u && i > 1);
    if (v < 0) b[--i] = '-';
    put(b + i);
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("SHELL-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static void cat(wchar_t *to, const wchar_t *from) {
    to += wlen(to);
    while ((*to++ = *from++) != 0) {
    }
}
static void copy(wchar_t *to, const wchar_t *from) {
    to[0] = 0;
    cat(to, from);
}
static int exists(const wchar_t *path) {
    return GetFileAttributesW(path) != INVALID_FILE_ATTRIBUTES;
}
static int make_file(const wchar_t *path) {
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return 0;
    DWORD w;
    WriteFile(h, "door", 4, &w, NULL);
    CloseHandle(h);
    return w == 4;
}
/* Write `data` to `path` (a file or a `file:stream`). */
static int write_all(const wchar_t *path, const char *data) {
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return 0;
    DWORD w = 0, n = (DWORD)length(data);
    BOOL ok = WriteFile(h, data, n, &w, NULL);
    CloseHandle(h);
    return ok && w == n;
}
/* Whether `path` holds exactly `data`. */
static int holds(const wchar_t *path, const char *data) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return 0;
    char got[256];
    DWORD n = 0;
    BOOL ok = ReadFile(h, got, sizeof got, &n, NULL);
    CloseHandle(h);
    int same_bytes = ok && n == (DWORD)length(data);
    for (DWORD i = 0; same_bytes && i < n; i++) same_bytes = got[i] == data[i];
    return same_bytes;
}
static const char zone_internet[] =
    "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://www.example.com/\r\nHostUrl=https://www.example.com/downloaded.exe\r\n";
/* shellprobe.exe mark: this program as a download (Downloads\downloaded.exe
 * with its Zone.Identifier stream), for File Explorer's security warning. */
static UINT mark(void) {
    wchar_t self[MAX_PATH], target[MAX_PATH + 40], stream[MAX_PATH + 60];
    PWSTR downloads = NULL;
    if (!GetModuleFileNameW(NULL, self, MAX_PATH)) return 2;
    if (SHGetKnownFolderPath(&FOLDERID_Downloads, 0, NULL, &downloads) != S_OK) return 3;
    copy(target, downloads);
    CoTaskMemFree(downloads);
    cat(target, L"\\downloaded.exe");
    copy(stream, target);
    cat(stream, L":Zone.Identifier");
    int ok = CopyFileW(self, target, FALSE) && write_all(stream, zone_internet) && holds(stream, zone_internet);
    put(ok ? "SHELL-A mark PASSED\r\n" : "SHELL-A mark FAILED\r\n");
    return ok ? 0 : 4;
}
/* Double-NUL list from one path. */
static void list(wchar_t *to, const wchar_t *path) {
    copy(to, path);
    to[wlen(to) + 1] = 0;
}
static wchar_t self_path[MAX_PATH];
static volatile LONG thread_ran;
static DWORD WINAPI suspended_thread(LPVOID argument) {
    InterlockedExchange(&thread_ran, 1);
    return (DWORD)(ULONG_PTR)argument;
}
static DWORD wait_exit(HANDLE process) {
    DWORD code = 12345;
    if (process && WaitForSingleObject(process, 10000) == WAIT_OBJECT_0) GetExitCodeProcess(process, &code);
    return code;
}
/* Stack buffers this large need a probe the program has no runtime for. */
#define LONG_ARGUMENT 1500
static wchar_t temp[MAX_PATH], base[MAX_PATH], path[MAX_PATH], other[MAX_PATH], from[MAX_PATH + 2], to[MAX_PATH + 2], line[2048], text[MAX_PATH];

/* SHBrowseForFolder's callback: select the folder in lParam, then OK. */
static int CALLBACK browse_callback(HWND dialog, UINT message, LPARAM value, LPARAM data) {
    (void)value;
    if (message == BFFM_INITIALIZED) {
        SendMessageW(dialog, BFFM_SETSELECTIONW, TRUE, data);
        PostMessageW(dialog, WM_COMMAND, IDOK, 0);
    }
    return 0;
}
/* "shellprobe.exe tray" (guest only: on Windows it would leave the icon
 * in the user's notification-area settings): a notification icon whose
 * NIN_SELECT (NOTIFYICON_VERSION_4) ends the program. */
#define TRAY_MESSAGE (WM_APP + 1)
static UINT tray_selected;
static LRESULT CALLBACK tray_procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == TRAY_MESSAGE) {
        put("SHELL-A tray message ");
        number((long)LOWORD(lparam));
        put(" id ");
        number((long)HIWORD(lparam));
        put("\r\n");
        if (LOWORD(lparam) == NIN_SELECT && HIWORD(lparam) == 7) {
            tray_selected = 1;
            PostQuitMessage(0);
        }
        return 0;
    }
    if (message == WM_TIMER) {
        PostQuitMessage(1);
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}
/* shellprobe.exe recycle (guest only: it adds to the user's Recycle Bin):
 * SHFileOperation FO_DELETE with FOF_ALLOWUNDO moves a new file to the bin;
 * SHQueryRecycleBin counts it. */
static UINT recycle(void) {
    wchar_t file[MAX_PATH + 2];
    DWORD n = GetTempPathW(MAX_PATH, file);
    if (!n) return 2;
    cat(file, L"doors-recycle.doorsprobe");
    if (!make_file(file)) return 3;
    SHQUERYRBINFO before, after;
    memset(&before, 0, sizeof before);
    memset(&after, 0, sizeof after);
    before.cbSize = after.cbSize = sizeof before;
    HRESULT q1 = SHQueryRecycleBinW(NULL, &before);
    file[wlen(file) + 1] = 0;
    SHFILEOPSTRUCTW op;
    memset(&op, 0, sizeof op);
    op.wFunc = FO_DELETE;
    op.pFrom = file;
    op.fFlags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI;
    int d = SHFileOperationW(&op);
    HRESULT q2 = SHQueryRecycleBinW(NULL, &after);
    int ok = q1 == S_OK && q2 == S_OK && d == 0 && !exists(file) && after.i64NumItems == before.i64NumItems + 1;
    if (!ok) {
        /* Which part failed: the queries, the delete, the file, the count. */
        put("SHELL-A recycle detail q1 ");
        number((long)q1);
        put(" q2 ");
        number((long)q2);
        put(" delete ");
        number(d);
        put(" exists ");
        number(exists(file));
        put(" items ");
        number((long)before.i64NumItems);
        put(" ");
        number((long)after.i64NumItems);
        put("\r\n");
    }
    put(ok ? "SHELL-A recycle PASSED\r\n" : "SHELL-A recycle FAILED\r\n");
    return ok ? 0 : 4;
}
/* shellprobe.exe openwith | openas (guest only: on Windows it opens the
 * "How do you want to open this file?" picker): ShellExecuteEx of a file
 * of a type nothing is registered for. Without UI it fails with
 * ERROR_NO_ASSOCIATION; with UI the shell asks which app to use (the
 * desktop then shows its picker). "openas" asks even for a type that
 * has an app. */
static UINT open_with(int choose) {
    wchar_t file[MAX_PATH + 2];
    DWORD n = GetTempPathW(MAX_PATH, file);
    if (!n) return 2;
    cat(file, L"doors-open-with.doorsopen");
    if (!choose && !make_file(file)) return 3;
    SHELLEXECUTEINFOW info;
    memset(&info, 0, sizeof info);
    info.cbSize = sizeof info;
    info.lpFile = file;
    info.nShow = SW_SHOWNORMAL;
    if (choose) {
        info.lpVerb = L"openas";
    } else {
        info.fMask = SEE_MASK_FLAG_NO_UI;
        BOOL quiet = ShellExecuteExW(&info);
        DWORD error = GetLastError();
        put("SHELL-A openwith quiet ");
        number(quiet);
        put(" error ");
        number((long)error);
        put("\r\n");
        if (quiet || error != ERROR_NO_ASSOCIATION) {
            put("SHELL-A openwith FAILED\r\n");
            return 4;
        }
        info.fMask = 0;
    }
    BOOL asked = ShellExecuteExW(&info);
    put(asked && (INT_PTR)info.hInstApp > 32 ? (choose ? "SHELL-A openas asked\r\n" : "SHELL-A openwith asked\r\n")
                                             : "SHELL-A openwith FAILED\r\n");
    return asked ? 0 : 5;
}
static UINT tray(void) {
    WNDCLASSW wc;
    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = tray_procedure;
    wc.hInstance = GetModuleHandleW(NULL);
    wc.lpszClassName = L"DoorsTrayProbe";
    RegisterClassW(&wc);
    HWND window = CreateWindowExW(0, L"DoorsTrayProbe", L"Tray probe", WS_OVERLAPPED, 0, 0, 100, 100, NULL, NULL, wc.hInstance, NULL);
    NOTIFYICONDATAW nid;
    memset(&nid, 0, sizeof nid);
    nid.cbSize = sizeof nid;
    nid.hWnd = window;
    nid.uID = 7;
    nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    nid.uCallbackMessage = TRAY_MESSAGE;
    nid.hIcon = LoadIconW(NULL, IDI_INFORMATION);
    copy(nid.szTip, L"Doors tray probe");
    BOOL added = window && Shell_NotifyIconW(NIM_ADD, &nid);
    nid.uVersion = NOTIFYICON_VERSION_4;
    BOOL versioned = added && Shell_NotifyIconW(NIM_SETVERSION, &nid);
    put(added && versioned ? "SHELL-A tray added\r\n" : "SHELL-A tray FAILED\r\n");
    if (!added || !versioned) return 2;
    SetTimer(window, 1, 60000, NULL);
    MSG m;
    while (GetMessageW(&m, NULL, 0, 0) > 0) {
        TranslateMessage(&m);
        DispatchMessageW(&m);
    }
    Shell_NotifyIconW(NIM_DELETE, &nid);
    put(tray_selected ? "SHELL-A tray PASSED\r\n" : "SHELL-A tray FAILED timeout\r\n");
    return tray_selected ? 0 : 3;
}
void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    int argc = 0;
    LPWSTR *argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    /* Run as Downloads\downloaded.exe (File Explorer, after its security
     * warning): exit 15 at once. */
    {
        static wchar_t self[MAX_PATH];
        DWORD n = GetModuleFileNameW(NULL, self, MAX_PATH);
        static const wchar_t tail[] = L"\\downloaded.exe";
        int k = wlen(tail);
        if (n > (DWORD)k && same(self + n - k, tail)) ExitProcess(15);
    }
    if (argv && argc == 2 && same(argv[1], L"mark")) {
        LocalFree(argv);
        ExitProcess(mark());
    }
    if (argv && argc == 2 && same(argv[1], L"recycle")) {
        LocalFree(argv);
        ExitProcess(recycle());
    }
    if (argv && argc == 2 && same(argv[1], L"tray")) {
        LocalFree(argv);
        ExitProcess(tray());
    }
    if (argv && argc == 2 && (same(argv[1], L"openwith") || same(argv[1], L"openas"))) {
        int choose = same(argv[1], L"openas");
        LocalFree(argv);
        ExitProcess(open_with(choose));
    }
    /* child env: 13 when lpEnvironment replaced the whole block (DOORS_PROBE
     * set, USERNAME gone), else 98. */
    if (argv && argc == 3 && same(argv[1], L"child") && same(argv[2], L"env")) {
        wchar_t value[32];
        DWORD got = GetEnvironmentVariableW(L"DOORS_PROBE", value, 32);
        DWORD user = GetEnvironmentVariableW(L"USERNAME", NULL, 0);
        LocalFree(argv);
        ExitProcess(got && same(value, L"suspended") && user == 0 ? 13 : 98);
    }
    /* child pipe: reads its standard input to the end, answers on its
     * standard output; 14 when both are pipes, else 98. */
    if (argv && argc == 3 && same(argv[1], L"child") && same(argv[2], L"pipe")) {
        LocalFree(argv);
        HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
        char data[64];
        DWORD got = 0, total = 0, done = 0;
        while (total < sizeof data && ReadFile(in, data + total, sizeof data - total, &got, NULL) && got) total += got;
        int piped = GetFileType(in) == FILE_TYPE_PIPE && GetFileType(out) == FILE_TYPE_PIPE;
        WriteFile(out, "pong:", 5, &done, NULL);
        WriteFile(out, data, total, &done, NULL);
        ExitProcess(piped ? 14 : 98);
    }
    if (argv && (argc == 3 || argc == 4) && same(argv[1], L"child")) {
        UINT code = 0;
        for (const wchar_t *p = argv[2]; *p >= L'0' && *p <= L'9'; p++) code = code * 10 + (UINT)(*p - L'0');
        /* A long third argument must arrive whole: LONG_ARGUMENT x's. */
        if (argc == 4) {
            int whole = wlen(argv[3]) == LONG_ARGUMENT;
            for (const wchar_t *p = argv[3]; *p; p++) whole = whole && *p == L'x';
            int line_ok = wlen(GetCommandLineW()) > LONG_ARGUMENT && length(GetCommandLineA()) > LONG_ARGUMENT;
            if (!whole || !line_ok) code = 99;
        }
        LocalFree(argv);
        ExitProcess(code);
    }
    if (argv) LocalFree(argv);
    GetModuleFileNameW(NULL, self_path, MAX_PATH);

    /* 1: CommandLineToArgvW: the program name's own rule, escapes, quotes. */
    int n = 0;
    LPWSTR *a = CommandLineToArgvW(L"\"C:\\Program Files\\x.exe\" a\\\\\\\"b \"two words\" c\"\"d", &n);
    int split = a && n == 4 && same(a[0], L"C:\\Program Files\\x.exe") && same(a[1], L"a\\\"b") && same(a[2], L"two words") && same(a[3], L"cd");
    if (a) LocalFree(a);
    step(1, "argv", split, (long)n);

    /* 2: known folders agree with the environment the session set. */
    PWSTR profile = NULL, local = NULL, none = NULL;
    HRESULT p = SHGetKnownFolderPath(&folder_profile, 0, NULL, &profile);
    HRESULT l = SHGetKnownFolderPath(&folder_local, KF_FLAG_CREATE, NULL, &local);
    HRESULT u = SHGetKnownFolderPath(&folder_unknown, 0, NULL, &none);
    wchar_t env[MAX_PATH];
    int profile_ok = p == S_OK && GetEnvironmentVariableW(L"USERPROFILE", env, MAX_PATH) && same(profile, env);
    int local_ok = l == S_OK && GetEnvironmentVariableW(L"LOCALAPPDATA", env, MAX_PATH) && same(local, env);
    HRESULT f1 = SHGetFolderPathW(NULL, CSIDL_APPDATA | CSIDL_FLAG_CREATE, NULL, 0, path);
    int roaming_ok = f1 == S_OK && GetEnvironmentVariableW(L"APPDATA", env, MAX_PATH) && same(path, env);
    HRESULT f2 = SHGetFolderPathW(NULL, CSIDL_PROGRAM_FILES, NULL, 0, path);
    HRESULT f3 = SHGetFolderPathW(NULL, 0x3F, NULL, 0, path);
    int profile_files = f2 == S_OK && same(path, L"C:\\Program Files");
    int unknown = u == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND) && none == NULL;
    /* The detail says which check failed: one bit each. */
    long folders = (long)(profile_ok | local_ok << 1 | roaming_ok << 2 | unknown << 3 | profile_files << 4 | (f3 == E_INVALIDARG) << 5 |
                          exists(L"C:\\Windows") << 6 | exists(L"C:\\Program Files") << 7);
    step(2, "known-folders", folders == 255, folders);

    /* 3: an item ID list for the profile folder gives its path back. */
    PIDLIST_ABSOLUTE item = NULL;
    HRESULT loc = SHGetFolderLocation(NULL, CSIDL_PROFILE, NULL, 0, &item);
    int back = item && SHGetPathFromIDListW(item, path) && profile && same(path, profile);
    UINT size = item ? ILGetSize(item) : 0;
    PIDLIST_ABSOLUTE clone = item ? ILClone(item) : NULL;
    int cloned = clone && ILGetSize(clone) == size && SHGetPathFromIDListW(clone, other) && same(other, path);
    ILFree(clone);
    ILFree(item);
    PIDLIST_ABSOLUTE desktop = NULL;
    SHGetFolderLocation(NULL, CSIDL_DESKTOP, NULL, 0, &desktop);
    int root = desktop && ILGetSize(desktop) == 2;
    ILFree(desktop);
    step(3, "item-ids", loc == S_OK && back && size > 2 && cloned && root, (long)size);
    CoTaskMemFree(profile);
    CoTaskMemFree(local);

    /* 4: SHCreateDirectoryEx makes every missing level, once. */
    GetTempPathW(MAX_PATH, temp);
    copy(base, temp);
    cat(base, L"doors-shell-a");
    SHFileOperationW(&(SHFILEOPSTRUCTW){NULL, FO_DELETE, (list(from, base), from), NULL, FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI, FALSE, NULL, NULL});
    copy(path, base);
    cat(path, L"\\src\\deep\\er");
    int made = SHCreateDirectoryExW(NULL, path, NULL);
    int again = SHCreateDirectoryExW(NULL, path, NULL);
    step(4, "create-directory", made == ERROR_SUCCESS && again == ERROR_ALREADY_EXISTS && exists(path), (long)made);

    /* 5: SHFileOperation: copy a tree, rename, move, delete. */
    copy(path, base);
    cat(path, L"\\src\\one.doorsprobe");
    int files = make_file(path);
    copy(path, base);
    cat(path, L"\\src\\deep\\two.doorsprobe");
    files = files && make_file(path);
    SHFILEOPSTRUCTW op;
    memset(&op, 0, sizeof op);
    op.fFlags = FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_NOCONFIRMMKDIR;
    copy(path, base);
    cat(path, L"\\src\\*");
    list(from, path);
    copy(path, base);
    cat(path, L"\\dst");
    list(to, path);
    op.wFunc = FO_COPY;
    op.pFrom = from;
    op.pTo = to;
    int c = SHFileOperationW(&op);
    copy(path, base);
    cat(path, L"\\dst\\deep\\two.doorsprobe");
    int copied = exists(path);
    copy(other, base);
    cat(other, L"\\dst\\deep\\three.doorsprobe");
    list(from, path);
    list(to, other);
    op.wFunc = FO_RENAME;
    int r = SHFileOperationW(&op);
    int renamed = !exists(path) && exists(other);
    copy(path, base);
    cat(path, L"\\dst\\one.doorsprobe");
    list(from, path);
    copy(other, base);
    list(to, other);
    op.wFunc = FO_MOVE;
    int m = SHFileOperationW(&op);
    copy(other, base);
    cat(other, L"\\one.doorsprobe");
    int moved = !exists(path) && exists(other);
    copy(path, base);
    cat(path, L"\\dst");
    list(from, path);
    op.wFunc = FO_DELETE;
    op.pTo = NULL;
    int d = SHFileOperationW(&op);
    int deleted = !exists(path);
    step(5, "file-operation", files && c == 0 && copied && r == 0 && renamed && m == 0 && moved && d == 0 && deleted && !op.fAnyOperationsAborted,
         (long)c * 1000 + r * 100 + m * 10 + d);

    /* 6: SHGetFileInfo names an unregistered type after its extension. */
    SHFILEINFOW info;
    memset(&info, 0, sizeof info);
    copy(path, base);
    cat(path, L"\\one.doorsprobe");
    DWORD_PTR got = SHGetFileInfoW(path, 0, &info, sizeof info, SHGFI_DISPLAYNAME | SHGFI_TYPENAME | SHGFI_ATTRIBUTES);
    int file_ok = got && same(info.szDisplayName, L"one.doorsprobe") && same(info.szTypeName, L"DOORSPROBE File") && (info.dwAttributes & SFGAO_FILESYSTEM) &&
                  !(info.dwAttributes & SFGAO_FOLDER);
    memset(&info, 0, sizeof info);
    copy(path, base);
    cat(path, L"\\src");
    got = SHGetFileInfoW(path, 0, &info, sizeof info, SHGFI_TYPENAME | SHGFI_ATTRIBUTES);
    int folder_ok = got && same(info.szTypeName, L"File folder") && (info.dwAttributes & SFGAO_FOLDER);
    memset(&info, 0, sizeof info);
    got = SHGetFileInfoW(L"nowhere.doorsprobe", FILE_ATTRIBUTE_NORMAL, &info, sizeof info, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES);
    int pretend = got && same(info.szTypeName, L"DOORSPROBE File");
    DWORD_PTR kind = SHGetFileInfoW(self_path, 0, NULL, 0, SHGFI_EXETYPE);
    step(6, "file-info", file_ok && folder_ok && pretend && kind == 0x4550, (long)kind);

    /* 7: ShellExecuteEx of this program: its exit code comes back. */
    SHELLEXECUTEINFOW sei;
    memset(&sei, 0, sizeof sei);
    sei.cbSize = sizeof sei;
    sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    sei.lpVerb = L"open";
    sei.lpFile = self_path;
    sei.lpParameters = L"child 7";
    sei.nShow = SW_HIDE;
    BOOL ran = ShellExecuteExW(&sei);
    DWORD seven = wait_exit(sei.hProcess);
    if (sei.hProcess) CloseHandle(sei.hProcess);
    /* ShellExecute (no process handle) succeeds with a value above 32. */
    HINSTANCE plain = ShellExecuteW(NULL, NULL, self_path, L"child 0", NULL, SW_HIDE);
    step(7, "shell-execute", ran && seven == 7 && (INT_PTR)plain > 32, (long)seven);

    /* 8: CreateProcess: a quoted program name, the exit code, a process ID;
     *    a missing program. */
    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof si);
    memset(&pi, 0, sizeof pi);
    si.cb = sizeof si;
    copy(line, L"\"");
    cat(line, self_path);
    cat(line, L"\" child 9");
    BOOL created = CreateProcessW(NULL, line, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi);
    DWORD id = created ? GetProcessId(pi.hProcess) : 0;
    DWORD nine = wait_exit(pi.hProcess);
    if (pi.hThread) CloseHandle(pi.hThread);
    if (pi.hProcess) CloseHandle(pi.hProcess);
    copy(text, L"doors-no-such-program-x.exe");
    BOOL missing = CreateProcessW(NULL, text, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi);
    DWORD error = GetLastError();
    step(8, "create-process", created && id != 0 && nine == 9 && !missing && error == ERROR_FILE_NOT_FOUND, (long)nine);

    /* 9: a command line longer than the startup record (1500 units of
     *    argument): the child sees every character. */
    copy(line, L"\"");
    cat(line, self_path);
    cat(line, L"\" child 11 ");
    int at = wlen(line);
    for (int i = 0; i < LONG_ARGUMENT; i++) line[at + i] = L'x';
    line[at + LONG_ARGUMENT] = 0;
    memset(&pi, 0, sizeof pi);
    BOOL long_created = CreateProcessW(NULL, line, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi);
    DWORD eleven = wait_exit(pi.hProcess);
    if (pi.hThread) CloseHandle(pi.hThread);
    if (pi.hProcess) CloseHandle(pi.hProcess);
    step(9, "long-command-line", long_created && eleven == 11, (long)eleven);

    /* 10: icons: the system error icon drawn (red disc, white cross), its
     *     bitmaps, a file type's small icon, a stock icon, the system
     *     image list, and a program without icon resources. */
    HICON error_icon = LoadIconW(NULL, IDI_ERROR);
    ICONINFO ii;
    memset(&ii, 0, sizeof ii);
    int icon_info = error_icon && GetIconInfo(error_icon, &ii) && ii.fIcon && ii.hbmColor && ii.hbmMask;
    if (ii.hbmColor) DeleteObject(ii.hbmColor);
    if (ii.hbmMask) DeleteObject(ii.hbmMask);
    BITMAPINFO bmi;
    memset(&bmi, 0, sizeof bmi);
    bmi.bmiHeader.biSize = sizeof bmi.bmiHeader;
    bmi.bmiHeader.biWidth = 32;
    bmi.bmiHeader.biHeight = -32;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    DWORD *bits = NULL;
    HDC dc = CreateCompatibleDC(NULL);
    HBITMAP dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, (void **)&bits, NULL, 0);
    HGDIOBJ previous = SelectObject(dc, dib);
    for (int i = 0; dib && i < 32 * 32; i++) bits[i] = 0x00ffffff;
    int drawn = dib && DrawIconEx(dc, 0, 0, error_icon, 32, 32, 0, NULL, DI_NORMAL);
    GdiFlush();
    DWORD centre = bits ? bits[16 * 32 + 16] : 0, side = bits ? bits[16 * 32 + 5] : 0;
    int white = (centre >> 16 & 255) > 160 && (centre >> 8 & 255) > 160 && (centre & 255) > 160;
    int red = (side >> 16 & 255) > 150 && (side >> 8 & 255) < 110 && (side & 255) < 110;
    SelectObject(dc, previous);
    DeleteObject(dib);
    DeleteDC(dc);
    copy(path, base);
    cat(path, L"\\notes.txt");
    make_file(path);
    SHFILEINFOW sfi;
    memset(&sfi, 0, sizeof sfi);
    DWORD_PTR small_ok = SHGetFileInfoW(path, 0, &sfi, sizeof sfi, SHGFI_ICON | SHGFI_SMALLICON);
    int small_width = 0;
    if (small_ok && sfi.hIcon) {
        ICONINFO small_info;
        BITMAP bm;
        if (GetIconInfo(sfi.hIcon, &small_info)) {
            if (small_info.hbmColor && GetObjectW(small_info.hbmColor, sizeof bm, &bm)) small_width = bm.bmWidth;
            if (small_info.hbmColor) DeleteObject(small_info.hbmColor);
            if (small_info.hbmMask) DeleteObject(small_info.hbmMask);
        }
        DestroyIcon(sfi.hIcon);
    }
    SHSTOCKICONINFO stock;
    memset(&stock, 0, sizeof stock);
    stock.cbSize = sizeof stock;
    HRESULT stocked = SHGetStockIconInfo(SIID_FOLDER, SHGSI_ICON | SHGSI_SYSICONINDEX, &stock);
    int stock_ok = stocked == S_OK && stock.hIcon && stock.iSysImageIndex >= 0;
    if (stock.hIcon) DestroyIcon(stock.hIcon);
    memset(&sfi, 0, sizeof sfi);
    HIMAGELIST system = (HIMAGELIST)SHGetFileInfoW(path, 0, &sfi, sizeof sfi, SHGFI_SYSICONINDEX);
    int listed = system && sfi.iIcon >= 0 && ImageList_GetImageCount(system) > sfi.iIcon;
    UINT own = ExtractIconExW(self_path, -1, NULL, NULL, 0);
    /* The detail names what failed: one bit per check. */
    long icon_failures = !icon_info | !drawn << 1 | !white << 2 | !red << 3 | (small_width != 16) << 4 |
                   !stock_ok << 5 | !listed << 6 | (own != 0) << 7;
    step(10, "icons", icon_failures == 0, icon_failures);

    /* 11: SHBrowseForFolder: the callback selects the probe's folder and
     *     presses OK; the folder comes back as an item ID list. */
    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    wchar_t chosen_name[MAX_PATH];
    chosen_name[0] = 0;
    BROWSEINFOW bi;
    memset(&bi, 0, sizeof bi);
    bi.pszDisplayName = chosen_name;
    bi.lpszTitle = L"Choose the probe folder";
    bi.ulFlags = BIF_RETURNONLYFSDIRS;
    bi.lpfn = browse_callback;
    bi.lParam = (LPARAM)base;
    PIDLIST_ABSOLUTE picked = SHBrowseForFolderW(&bi);
    other[0] = 0;
    int browsed = picked && SHGetPathFromIDListW(picked, other) && same(other, base) && same(chosen_name, L"doors-shell-a");
    if (picked) CoTaskMemFree(picked);
    step(11, "browse-folder", browsed, (long)wlen(other));

    /* 12: CreateProcess with CREATE_SUSPENDED and its own environment
     *     block: the child has not run (no exit) until ResumeThread, then
     *     sees only that block. */
    copy(line, L"\"");
    cat(line, self_path);
    cat(line, L"\" child env");
    static wchar_t block[] = L"DOORS_PROBE=suspended\0SystemRoot=C:\\Windows\0";
    memset(&pi, 0, sizeof pi);
    BOOL held = CreateProcessW(NULL, line, NULL, NULL, FALSE, CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                               block, NULL, &si, &pi);
    DWORD before = held ? WaitForSingleObject(pi.hProcess, 300) : 0;
    DWORD resumed = held ? ResumeThread(pi.hThread) : (DWORD)-1;
    DWORD thirteen = held ? wait_exit(pi.hProcess) : 0;
    if (pi.hThread) CloseHandle(pi.hThread);
    if (pi.hProcess) CloseHandle(pi.hProcess);
    step(12, "suspended-process", held && before == WAIT_TIMEOUT && resumed == 1 && thirteen == 13, (long)thirteen);

    /* 13: CreateThread CREATE_SUSPENDED waits for ResumeThread;
     *     SuspendThread and ResumeThread count. */
    DWORD tid = 0;
    HANDLE t = CreateThread(NULL, 0, suspended_thread, (LPVOID)21, CREATE_SUSPENDED, &tid);
    Sleep(100);
    LONG early = thread_ran;
    DWORD nested = t ? SuspendThread(t) : (DWORD)-1; /* 1 -> 2 */
    DWORD first = t ? ResumeThread(t) : (DWORD)-1;   /* 2 -> 1 */
    Sleep(50);
    LONG still = thread_ran;
    DWORD second = t ? ResumeThread(t) : (DWORD)-1;  /* 1 -> 0: runs */
    DWORD joined = t ? WaitForSingleObject(t, 5000) : WAIT_FAILED;
    DWORD code = 0;
    if (t) {
        GetExitCodeThread(t, &code);
        CloseHandle(t);
    }
    step(13, "suspended-thread", t && tid && !early && nested == 1 && first == 2 && !still && second == 1 &&
                                     joined == WAIT_OBJECT_0 && thread_ran && code == 21,
         (long)code);

    /* 14: anonymous pipes as a child's standard input and output
     *     (STARTF_USESTDHANDLES, bInheritHandles): the child answers what
     *     it read; once it has exited the parent's read ends with
     *     ERROR_BROKEN_PIPE. */
    HANDLE in_r = NULL, in_w = NULL, out_r = NULL, out_w = NULL;
    BOOL piped = CreatePipe(&in_r, &in_w, NULL, 0) && CreatePipe(&out_r, &out_w, NULL, 0);
    DWORD pipe_kind = piped ? GetFileType(out_r) : 0;
    static STARTUPINFOW psi;
    memset(&psi, 0, sizeof psi);
    psi.cb = sizeof psi;
    psi.dwFlags = STARTF_USESTDHANDLES;
    psi.hStdInput = in_r;
    psi.hStdOutput = out_w;
    psi.hStdError = out_w;
    copy(line, L"\"");
    cat(line, self_path);
    cat(line, L"\" child pipe");
    memset(&pi, 0, sizeof pi);
    BOOL spawned = piped && CreateProcessW(NULL, line, NULL, NULL, TRUE, 0, NULL, NULL, &psi, &pi);
    if (in_r) CloseHandle(in_r);
    if (out_w) CloseHandle(out_w);
    DWORD sent = 0;
    BOOL wrote = spawned && WriteFile(in_w, "ping", 4, &sent, NULL);
    if (in_w) CloseHandle(in_w);
    char reply[64];
    DWORD chunk = 0, total = 0;
    while (spawned && total < sizeof reply - 1 && ReadFile(out_r, reply + total, sizeof reply - 1 - total, &chunk, NULL) && chunk)
        total += chunk;
    DWORD end_error = GetLastError();
    DWORD fourteen = spawned ? wait_exit(pi.hProcess) : 0;
    if (pi.hThread) CloseHandle(pi.hThread);
    if (pi.hProcess) CloseHandle(pi.hProcess);
    if (out_r) CloseHandle(out_r);
    static const char expected[] = "pong:ping";
    int answered = total == sizeof expected - 1;
    for (DWORD i = 0; answered && i < total; i++) answered = reply[i] == expected[i];
    step(14, "pipe", piped && pipe_kind == FILE_TYPE_PIPE && spawned && wrote && sent == 4 && answered &&
                         end_error == ERROR_BROKEN_PIPE && fourteen == 14,
         (long)(fourteen * 1000 + total));

    /* 15: alternate data streams: a Zone.Identifier stream is written and
     *     read back; FindFirstStreamW lists ::$DATA then the stream;
     *     CopyFile carries it; DeleteFile of the stream leaves the file. */
    static wchar_t sfile[MAX_PATH + 40], sstream[MAX_PATH + 60], scopy[MAX_PATH + 40], scopy_stream[MAX_PATH + 60];
    copy(sfile, base);
    cat(sfile, L"\\motw.doorsprobe");
    copy(sstream, sfile);
    cat(sstream, L":Zone.Identifier");
    copy(scopy, base);
    cat(scopy, L"\\motw-copy.doorsprobe");
    copy(scopy_stream, scopy);
    cat(scopy_stream, L":Zone.Identifier:$DATA");
    int written = make_file(sfile) && write_all(sstream, zone_internet);
    int read_back = holds(sstream, zone_internet) && holds(sfile, "door");
    static WIN32_FIND_STREAM_DATA sd;
    HANDLE sh = FindFirstStreamW(sfile, FindStreamInfoStandard, &sd, 0);
    int enumerated = sh != INVALID_HANDLE_VALUE && same(sd.cStreamName, L"::$DATA") && sd.StreamSize.QuadPart == 4;
    enumerated = enumerated && FindNextStreamW(sh, &sd) && same(sd.cStreamName, L":Zone.Identifier:$DATA") &&
             sd.StreamSize.QuadPart == length(zone_internet);
    enumerated = enumerated && !FindNextStreamW(sh, &sd) && GetLastError() == ERROR_HANDLE_EOF;
    if (sh != INVALID_HANDLE_VALUE) FindClose(sh);
    int carried = CopyFileW(sfile, scopy, FALSE) && holds(scopy_stream, zone_internet);
    int removed = DeleteFileW(scopy_stream) && !holds(scopy_stream, zone_internet) &&
                  GetLastError() == ERROR_FILE_NOT_FOUND && holds(scopy, "door");
    step(15, "streams", written && read_back && enumerated && carried && removed,
         (long)written * 10000 + (long)read_back * 1000 + (long)enumerated * 100 + (long)carried * 10 + (long)removed);

    /* Clean up the temporary folder. */
    list(from, base);
    SHFileOperationW(&(SHFILEOPSTRUCTW){NULL, FO_DELETE, from, NULL, FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI, FALSE, NULL, NULL});
    put(failed_step ? "SHELL-A FAILED\r\n" : "SHELL-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

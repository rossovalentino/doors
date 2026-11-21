/* INSTALL-A: msi.dll's API from an ordinary Windows console program
 * (Windows SDK, no C runtime; Msi.Lib imports by ordinal).
 *
 *   msiprobe.exe api <package>      databases, views with parameters,
 *                                   records, column information, primary
 *                                   keys and summary information
 *   msiprobe.exe install <package>  MsiInstallProduct without UI, then the
 *                                   product queries (state, information,
 *                                   enumeration, related products,
 *                                   component path)
 *   msiprobe.exe remove             MsiConfigureProductEx to absent
 *
 * One "INSTALL-A <mode> <n> <name> ok|FAILED <detail>" line per check, then
 * "INSTALL-A <mode> PASSED" or "FAILED"; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <msi.h>
#include <msiquery.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    unsigned char *d = destination;
    while (count--) *d++ = (unsigned char)value;
    return destination;
}

#define PRODUCT L"{3D1C9A57-0001-4F60-9C14-7A5B2E0D6F11}"
#define UPGRADE L"{3D1C9A57-8E2B-4F60-9C14-7A5B2E0D6F11}"
#define TOOL_COMPONENT L"{2C6B1A00-1111-4A2B-9C3D-4E5F60718293}"

static HANDLE out;
static const char *mode_name;
static int failed;
static void put(const char *s) {
    DWORD n = 0, w = 0;
    while (s[n]) n++;
    WriteFile(out, s, n, &w, NULL);
}
static void number(long v) {
    char b[24];
    int i = 23;
    unsigned long u = v < 0 ? (unsigned long)-v : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (v < 0) b[--i] = '-';
    put(b + i);
}
static void step(int id, const char *name, int ok, long detail) {
    put("INSTALL-A ");
    put(mode_name);
    put(" ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok) failed = 1;
}
static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static int ends_with(const wchar_t *a, const wchar_t *tail) {
    int n = 0, m = 0;
    while (a[n]) n++;
    while (tail[m]) m++;
    return n >= m && same(a + n - m, tail);
}
static int split(wchar_t *line, wchar_t **words, int capacity) {
    int count = 0;
    while (*line && count < capacity) {
        while (*line == L' ') line++;
        if (!*line) break;
        wchar_t *start = line, *o = line;
        int quoted = 0;
        while (*line && (quoted || *line != L' ')) {
            if (*line == L'"') {
                quoted = !quoted;
                line++;
                continue;
            }
            *o++ = *line++;
        }
        if (*line) line++;
        *o = 0;
        words[count++] = start;
    }
    return count;
}

static void api(const wchar_t *package) {
    MSIHANDLE db = 0, view = 0, record = 0, parameters = 0, info = 0, summary = 0, keys = 0;
    UINT r = MsiOpenDatabaseW(package, MSIDBOPEN_READONLY, &db);
    step(1, "open-database", r == ERROR_SUCCESS, (long)r);
    wchar_t text[128];
    DWORD length = 128;
    r = MsiDatabaseOpenViewW(db, L"SELECT `Value` FROM `Property` WHERE `Property` = 'ProductName'", &view);
    if (r == ERROR_SUCCESS) r = MsiViewExecute(view, 0);
    if (r == ERROR_SUCCESS) r = MsiViewFetch(view, &record);
    if (r == ERROR_SUCCESS) r = MsiRecordGetStringW(record, 1, text, &length);
    step(2, "view", r == ERROR_SUCCESS && same(text, L"Doors Tools") && MsiViewFetch(view, &info) == ERROR_NO_MORE_ITEMS, (long)r);
    MsiCloseHandle(record);
    MsiCloseHandle(view);
    /* A parameter, an integer column, the column names. */
    int count = 0, sizes = 0;
    r = MsiDatabaseOpenViewW(db, L"SELECT `File`, `FileSize` FROM `File` WHERE `Sequence` > ? ORDER BY `Sequence`", &view);
    parameters = MsiCreateRecord(1);
    MsiRecordSetInteger(parameters, 1, 2);
    if (r == ERROR_SUCCESS) r = MsiViewExecute(view, parameters);
    while (r == ERROR_SUCCESS && MsiViewFetch(view, &record) == ERROR_SUCCESS) {
        count++;
        sizes += MsiRecordGetInteger(record, 2);
        MsiCloseHandle(record);
    }
    UINT columns = MsiViewGetColumnInfo(view, MSICOLINFO_NAMES, &info);
    length = 128;
    if (columns == ERROR_SUCCESS) columns = MsiRecordGetStringW(info, 1, text, &length);
    step(3, "parameters", r == ERROR_SUCCESS && count == 2 && sizes > 0 && columns == ERROR_SUCCESS && same(text, L"File"),
         count * 1000 + (long)columns);
    MsiCloseHandle(info);
    MsiCloseHandle(parameters);
    MsiCloseHandle(view);
    r = MsiDatabaseGetPrimaryKeysW(db, L"Registry", &keys);
    length = 128;
    if (r == ERROR_SUCCESS) r = MsiRecordGetStringW(keys, 1, text, &length);
    step(4, "primary-keys", r == ERROR_SUCCESS && same(text, L"Registry") && MsiRecordGetFieldCount(keys) == 1, (long)r);
    MsiCloseHandle(keys);
    r = MsiGetSummaryInformationW(db, NULL, 0, &summary);
    UINT kind = 0;
    INT value = 0;
    FILETIME time;
    length = 128;
    if (r == ERROR_SUCCESS) r = MsiSummaryInfoGetPropertyW(summary, 3 /* PID_SUBJECT */, &kind, &value, &time, text, &length);
    UINT page = 0;
    INT pages = 0;
    if (r == ERROR_SUCCESS) MsiSummaryInfoGetPropertyW(summary, 14 /* PID_PAGECOUNT */, &page, &pages, &time, NULL, NULL);
    step(5, "summary", r == ERROR_SUCCESS && kind == 30 && same(text, L"Doors Tools") && pages == 500, (long)r);
    MsiCloseHandle(summary);
    r = MsiCloseHandle(db);
    step(6, "close", r == ERROR_SUCCESS && MsiCloseHandle(db) == ERROR_INVALID_HANDLE, (long)r);
}

static void install(const wchar_t *package) {
    MsiSetInternalUI(INSTALLUILEVEL_NONE, NULL);
    UINT r = MsiInstallProductW(package, L"");
    step(1, "install-product", r == ERROR_SUCCESS, (long)r);
    INSTALLSTATE state = MsiQueryProductStateW(PRODUCT);
    step(2, "product-state", state == INSTALLSTATE_DEFAULT, (long)state);
    wchar_t text[300];
    DWORD length = 300;
    r = MsiGetProductInfoW(PRODUCT, INSTALLPROPERTY_VERSIONSTRING, text, &length);
    step(3, "product-info", r == ERROR_SUCCESS && same(text, L"1.0.0"), (long)r);
    wchar_t code[39];
    int found = 0;
    for (DWORD i = 0; MsiEnumProductsW(i, code) == ERROR_SUCCESS && i < 64; i++) {
        if (same(code, PRODUCT)) found = 1;
    }
    step(4, "enum-products", found, 0);
    r = MsiEnumRelatedProductsW(UPGRADE, 0, 0, code);
    step(5, "related-products", r == ERROR_SUCCESS && same(code, PRODUCT), (long)r);
    length = 300;
    state = MsiGetComponentPathW(PRODUCT, TOOL_COMPONENT, text, &length);
    step(6, "component-path", state == INSTALLSTATE_LOCAL && ends_with(text, L"\\Doors Tools\\toolprobe.exe"), (long)state);
}

static void remove(void) {
    MsiSetInternalUI(INSTALLUILEVEL_NONE, NULL);
    UINT r = MsiConfigureProductExW(PRODUCT, 0, INSTALLSTATE_ABSENT, L"");
    step(1, "configure-absent", r == ERROR_SUCCESS, (long)r);
    INSTALLSTATE state = MsiQueryProductStateW(PRODUCT);
    step(2, "product-state", state == INSTALLSTATE_UNKNOWN, (long)state);
    r = MsiConfigureProductExW(PRODUCT, 0, INSTALLSTATE_ABSENT, L"");
    step(3, "unknown-product", r == ERROR_UNKNOWN_PRODUCT, (long)r);
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    static wchar_t line[2048];
    const wchar_t *full = GetCommandLineW();
    int n = 0;
    while (full[n] && n < 2047) {
        line[n] = full[n];
        n++;
    }
    line[n] = 0;
    wchar_t *words[6];
    int count = split(line, words, 6);
    if (count >= 3 && same(words[1], L"api")) {
        mode_name = "api";
        api(words[2]);
    } else if (count >= 3 && same(words[1], L"install")) {
        mode_name = "msiapi";
        install(words[2]);
    } else if (count >= 2 && same(words[1], L"remove")) {
        mode_name = "msiremove";
        remove();
    } else {
        put("usage: msiprobe api|install <package> | remove\r\n");
        ExitProcess(2);
    }
    put("INSTALL-A ");
    put(mode_name);
    put(failed ? " FAILED\r\n" : " PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

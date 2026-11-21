/* PATHS-A probe (P16 M5): long paths, up to MAX_PATH, from an ordinary
 * Windows console program (Windows SDK, no C runtime):
 *
 *   pathprobe.exe raw | check | guest
 *
 * `limits` only asks for the attributes of paths that do not exist, of
 * 240, 259, 260 and 300 characters under C:\Windows and under the user's
 * profile (nothing is created): its output is compared with what Windows 11
 * printed (`raw` on the owner's PC; `pathprobe.expected.h`). `guest` also
 * builds a folder tree in %TEMP% whose deepest file's path is 255
 * characters long, writes, reads, finds, renames and deletes in it, and
 * removes the tree, which only the guest does. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <intrin.h>

#pragma function(memset, memcpy)
void *memset(void *d, int v, size_t n) {
    __stosb((unsigned char *)d, (unsigned char)v, n);
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    __movsb((unsigned char *)d, (const unsigned char *)s, n);
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
static void num(long long v) {
    char b[32];
    int i = 31, neg = v < 0;
    unsigned long long u = neg ? (unsigned long long)-v : (unsigned long long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (neg) b[--i] = '-';
    put(b + i);
}
static void nl(void) { put("\r\n"); }
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static void append(wchar_t *to, int *n, const wchar_t *s) {
    while (*s) to[(*n)++] = *s++;
    to[*n] = 0;
}

/* `base` extended to exactly `length` characters with folders named
 * "dddd...\" (at most 40 characters each) and a last file name of x's. */
static void long_path(wchar_t *path, const wchar_t *base, int length) {
    int n = 0, i;
    append(path, &n, base);
    while (length - n > 41) {
        path[n++] = L'\\';
        for (i = 0; i < 39; i++) path[n++] = L'd';
        path[n] = 0;
    }
    path[n++] = L'\\';
    while (n < length) path[n++] = L'x';
    path[n] = 0;
}
static void limits_step(void) {
    static const int lengths[] = {240, 259, 260, 300};
    wchar_t path[400], profile[MAX_PATH];
    int i, b;
    DWORD n = GetEnvironmentVariableW(L"USERPROFILE", profile, MAX_PATH);
    for (b = 0; b < 2; b++) {
        const wchar_t *base = b == 0 ? L"C:\\Windows" : profile;
        if (b == 1 && (n == 0 || n >= MAX_PATH)) continue;
        for (i = 0; i < 4; i++) {
            DWORD a, e;
            long_path(path, base, lengths[i]);
            SetLastError(0);
            a = GetFileAttributesW(path);
            e = GetLastError();
            put(b == 0 ? "windows " : "profile ");
            num(wlen(path));
            put(" attributes=");
            num(a == INVALID_FILE_ATTRIBUTES ? -1 : (long long)a);
            put(" error=");
            num(e);
            {
                HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
                put(" open-error=");
                num(h == INVALID_HANDLE_VALUE ? GetLastError() : 0);
                if (h != INVALID_HANDLE_VALUE) CloseHandle(h);
            }
            nl();
        }
    }
}

static int failed_check, check;
static DWORD last;
#define EXPECT(c)                                    \
    do {                                             \
        check++;                                     \
        if (!(c)) {                                  \
            last = GetLastError();                   \
            if (!failed_check) failed_check = check; \
        }                                            \
    } while (0)
/* Guest only: a tree in %TEMP% whose deepest file's path is 255
 * characters; every folder is created, used and removed. */
static void deep_step(void) {
    static wchar_t temp[MAX_PATH], root[MAX_PATH], path[MAX_PATH], folders[8][MAX_PATH], renamed[MAX_PATH];
    int depth = 0, n = 0, i;
    DWORD t = GetTempPathW(MAX_PATH, temp);
    EXPECT(t > 0 && t < MAX_PATH);
    append(root, &n, temp);
    if (root[n - 1] == L'\\') root[--n] = 0;
    append(root, &n, L"\\DoorsPaths");
    CreateDirectoryW(root, NULL);
    /* Folders of at most 39 characters while more than 40 would be left for
     * the file's name, each leaving at least 12 for it (names short, the
     * path long, whatever %TEMP%'s length), then the file. */
    n = 0;
    append(path, &n, root);
    while (255 - n - 1 > 40 && depth < 8) {
        int room = 255 - n - 1 - 1 - 12, length = room < 39 ? room : 39;
        if (length < 1) break;
        path[n++] = L'\\';
        for (i = 0; i < length; i++) path[n++] = (wchar_t)(L'a' + depth);
        path[n] = 0;
        EXPECT(CreateDirectoryW(path, NULL));
        memcpy(folders[depth], path, (n + 1) * sizeof(wchar_t));
        depth++;
    }
    path[n++] = L'\\';
    while (n < 255 - 4) path[n++] = L'f';
    path[n] = 0;
    append(path, &n, L".txt");
    EXPECT(wlen(path) == 255);
    {
        HANDLE h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_NEW, 0, NULL);
        DWORD w = 0, r = 0;
        char b[16];
        EXPECT(h != INVALID_HANDLE_VALUE);
        EXPECT(WriteFile(h, "Doors long path", 15, &w, NULL) && w == 15);
        SetFilePointer(h, 0, NULL, FILE_BEGIN);
        EXPECT(ReadFile(h, b, sizeof b, &r, NULL) && r == 15 && b[0] == 'D' && b[14] == 'h');
        CloseHandle(h);
    }
    EXPECT(GetFileAttributesW(path) != INVALID_FILE_ATTRIBUTES);
    {
        WIN32_FIND_DATAW f;
        wchar_t pattern[MAX_PATH];
        int m = 0;
        HANDLE h;
        append(pattern, &m, folders[depth - 1]);
        append(pattern, &m, L"\\*.txt");
        h = FindFirstFileW(pattern, &f);
        EXPECT(h != INVALID_HANDLE_VALUE && wlen(f.cFileName) == 255 - wlen(folders[depth - 1]) - 1);
        if (h != INVALID_HANDLE_VALUE) FindClose(h);
    }
    /* A rename to a name one character shorter; then one past MAX_PATH. */
    n = 0;
    append(renamed, &n, path);
    renamed[n - 5] = L'.';
    renamed[n - 4] = L't';
    renamed[n - 3] = L'x';
    renamed[n - 2] = L't';
    renamed[n - 1] = 0;
    EXPECT(MoveFileW(path, renamed));
    EXPECT(GetFileAttributesW(path) == INVALID_FILE_ATTRIBUTES && GetLastError() == ERROR_FILE_NOT_FOUND);
    {
        wchar_t toolong[400];
        int m = 0;
        append(toolong, &m, renamed);
        while (m < 270) toolong[m++] = L'y';
        toolong[m] = 0;
        EXPECT(CreateFileW(toolong, GENERIC_WRITE, 0, NULL, CREATE_NEW, 0, NULL) == INVALID_HANDLE_VALUE);
    }
    EXPECT(DeleteFileW(renamed));
    for (i = depth - 1; i >= 0; i--) EXPECT(RemoveDirectoryW(folders[i]));
    EXPECT(RemoveDirectoryW(root));
}

/* Guest only: a registry key whose path is about 190 characters (past
 * one storage packet), created, given a value, read back and deleted. */
static void registry_step(void) {
    static wchar_t key[400];
    int n = 0, depth, i;
    HKEY h;
    DWORD type = 0, value = 0, size = sizeof value, set = 0x2026;
    append(key, &n, L"Software\\DoorsPaths");
    for (depth = 0; depth < 4; depth++) {
        key[n++] = L'\\';
        for (i = 0; i < 39; i++) key[n++] = (wchar_t)(L'k' + depth);
        key[n] = 0;
    }
    EXPECT(n > 150);
    EXPECT(RegCreateKeyExW(HKEY_CURRENT_USER, key, 0, NULL, 0, KEY_ALL_ACCESS, NULL, &h, NULL) ==
           ERROR_SUCCESS);
    EXPECT(RegSetValueExW(h, L"Depth", 0, REG_DWORD, (const BYTE *)&set, sizeof set) == ERROR_SUCCESS);
    RegCloseKey(h);
    EXPECT(RegGetValueW(HKEY_CURRENT_USER, key, L"Depth", RRF_RT_REG_DWORD, &type, &value, &size) ==
               ERROR_SUCCESS &&
           value == 0x2026 && type == REG_DWORD);
    EXPECT(RegDeleteTreeW(HKEY_CURRENT_USER, L"Software\\DoorsPaths") == ERROR_SUCCESS);
    EXPECT(RegOpenKeyExW(HKEY_CURRENT_USER, key, 0, KEY_READ, &h) == ERROR_FILE_NOT_FOUND);
}

struct step {
    const char *name;
    void (*run)(void);
    int guest_only;
};
static const struct step steps[] = {
    {"limits", limits_step, 0}, {"deep", deep_step, 1}, {"registry", registry_step, 1}};
static const char *const expected[] = {
#include "pathprobe.expected.h"
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[300];
    int n = line_length(s), k;
    if (n > 299) n = 299;
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
        if (steps[i].guest_only) {
            if (!guest || failed) continue;
            check = 0;
            failed_check = 0;
            steps[i].run();
            put("PATHS-A ");
            num(i + 1);
            put(" ");
            put(steps[i].name);
            if (failed_check) {
                put(" FAILED check ");
                num(failed_check);
                put(" error=");
                num(last);
                failed = 1;
            } else {
                put(" ok ");
                num(check);
            }
            nl();
            continue;
        }
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
            put("PATHS-A ");
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
    put(failed ? "PATHS-A FAILED\r\n" : "PATHS-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

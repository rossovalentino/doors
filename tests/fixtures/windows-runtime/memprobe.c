/* MEM-A probe (P18): the memory calls managed runtimes are built on, from
 * an ordinary Windows console program (Windows SDK, no C runtime):
 *
 *   memprobe.exe raw | check | guest
 *
 * `sections` double-maps a SEC_RESERVE pagefile section as CoreCLR's
 * executable allocator does (write through one view, run through the
 * other); `placeholders` splits, replaces and coalesces placeholders with
 * VirtualAlloc2 and MapViewOfFile3 as CoreCLR maps image parts; `copy`
 * checks FILE_MAP_COPY views copy on write; `image` maps kernel32.dll as a
 * SEC_IMAGE section. Their output is compared with what Windows 11 printed
 * (`raw` on the owner's PC: these create no files; `memprobe.expected.h`).
 * `files` (guest only) writes and reads a 1 MiB file in one call each and
 * changes it through a shared file mapping, then deletes it. */
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
static void hex(unsigned long long v) {
    char b[32];
    int i = 31;
    b[i] = 0;
    do {
        b[--i] = "0123456789abcdef"[v & 15];
        v >>= 4;
    } while (v);
    b[--i] = 'x';
    b[--i] = '0';
    put(b + i);
}
static void nl(void) { put("\r\n"); }
/* "name state=.. protect=.. type=.." of the region at `p`. */
static void region(const char *name, const void *p) {
    MEMORY_BASIC_INFORMATION m;
    put(name);
    if (!VirtualQuery(p, &m, sizeof m)) {
        put(" query-error=");
        num(GetLastError());
        nl();
        return;
    }
    put(" state=");
    hex(m.State);
    put(" protect=");
    hex(m.Protect);
    put(" type=");
    hex(m.Type);
    put(" allocation-protect=");
    hex(m.AllocationProtect);
    nl();
}
static void result(const char *name, long long ok) {
    put(name);
    put(ok ? " ok" : " failed");
    if (!ok) {
        put(" error=");
        num(GetLastError());
    }
    nl();
}

typedef PVOID(WINAPI *VirtualAlloc2Fn)(HANDLE, PVOID, SIZE_T, ULONG, ULONG, MEM_EXTENDED_PARAMETER *, ULONG);
typedef PVOID(WINAPI *MapViewOfFile3Fn)(HANDLE, HANDLE, PVOID, ULONG64, SIZE_T, ULONG, ULONG, MEM_EXTENDED_PARAMETER *,
                                        ULONG);
typedef BOOL(WINAPI *UnmapViewOfFile2Fn)(HANDLE, PVOID, ULONG);

/* CoreCLR's double mapping: a 1 GiB SEC_RESERVE section, an executable
 * view that commits, a writable view of the same pages. */
static void sections_step(void) {
    HANDLE section = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_EXECUTE_READWRITE | SEC_RESERVE, 0,
                                        0x40000000, NULL);
    unsigned char *rx, *rw;
    int (*code)(void);
    result("create", section != NULL);
    if (!section) return;
    rx = (unsigned char *)MapViewOfFile(section, FILE_MAP_EXECUTE | FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, 0x100000);
    result("map-rx", rx != NULL);
    if (!rx) return;
    region("reserved", rx);
    result("commit", VirtualAlloc(rx, 0x1000, MEM_COMMIT, PAGE_EXECUTE_READ) == rx);
    region("committed", rx);
    region("beyond", rx + 0x1000);
    rw = (unsigned char *)MapViewOfFile(section, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, 0x10000);
    result("map-rw", rw != NULL);
    if (!rw) return;
    region("writable", rw);
    region("writable-uncommitted", rw + 0x1000);
    /* mov eax, 42; ret */
    rw[0] = 0xB8, rw[1] = 42, rw[2] = 0, rw[3] = 0, rw[4] = 0, rw[5] = 0xC3;
    FlushInstructionCache(GetCurrentProcess(), rx, 6);
    code = (int (*)(void))(void *)rx;
    put("run=");
    num(code());
    nl();
    put("same-bytes=");
    num(rx[1] == 42);
    nl();
    result("unmap-rw", UnmapViewOfFile(rw));
    region("after-unmap", rw);
    result("unmap-rx-inside", UnmapViewOfFile(rx + 0x2000));
    result("close", CloseHandle(section));
}

static void placeholders_step(void) {
    HMODULE memory = LoadLibraryExW(L"api-ms-win-core-memory-l1-1-6.dll", NULL, LOAD_LIBRARY_SEARCH_SYSTEM32);
    VirtualAlloc2Fn alloc2 = memory ? (VirtualAlloc2Fn)(void *)GetProcAddress(memory, "VirtualAlloc2") : NULL;
    MapViewOfFile3Fn map3 = memory ? (MapViewOfFile3Fn)(void *)GetProcAddress(memory, "MapViewOfFile3") : NULL;
    UnmapViewOfFile2Fn unmap2 =
        memory ? (UnmapViewOfFile2Fn)(void *)GetProcAddress(memory, "UnmapViewOfFile2") : NULL;
    HANDLE section;
    unsigned char *p, *view, *rest;
    result("api-set", alloc2 && map3 && unmap2);
    if (!alloc2 || !map3 || !unmap2) return;
    p = (unsigned char *)alloc2(GetCurrentProcess(), NULL, 0x30000, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                                PAGE_NOACCESS, NULL, 0);
    result("placeholder", p != NULL);
    if (!p) return;
    region("placeholder", p);
    result("commit-placeholder-refused", VirtualAlloc(p, 0x1000, MEM_COMMIT, PAGE_READWRITE) == NULL);
    result("split", VirtualFree(p, 0x10000, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER));
    section = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 0x10000, NULL);
    view = (unsigned char *)map3(section, GetCurrentProcess(), p, 0, 0x10000, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE,
                                 NULL, 0);
    result("replace-view", view == p);
    if (view != p) return;
    view[0x1234] = 77;
    region("view", view);
    rest = (unsigned char *)alloc2(GetCurrentProcess(), p + 0x10000, 0x20000,
                                   MEM_RESERVE | MEM_COMMIT | MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, NULL, 0);
    result("replace-allocation", rest == p + 0x10000);
    if (rest != p + 0x10000) return;
    rest[0x1FFFF] = 5;
    region("allocation", rest);
    result("unmap-preserve", unmap2(GetCurrentProcess(), view, MEM_PRESERVE_PLACEHOLDER));
    region("preserved", p);
    result("free-preserve", VirtualFree(rest, 0x20000, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER));
    result("coalesce", VirtualFree(p, 0x30000, MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS));
    region("coalesced", p + 0x20000);
    result("release", VirtualFree(p, 0, MEM_RELEASE));
    region("released", p);
    CloseHandle(section);
}

static void copy_step(void) {
    HANDLE section = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 0x20000, NULL);
    unsigned char *a, *b;
    a = (unsigned char *)MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0);
    b = (unsigned char *)MapViewOfFile(section, FILE_MAP_COPY, 0, 0, 0);
    result("views", a && b);
    if (!a || !b) return;
    a[0] = 'A';
    a[0x10000] = 'S';
    region("copy-before", b);
    put("copy-sees=");
    num(b[0] == 'A');
    nl();
    b[0] = 'B';
    region("copy-after", b);
    region("copy-untouched", b + 0x10000);
    put("shared-kept=");
    num(a[0] == 'A');
    nl();
    a[0] = 'C';
    a[0x10000] = 'T';
    put("copy-kept=");
    num(b[0] == 'B');
    put(" copy-shared-page=");
    num(b[0x10000] == 'T');
    nl();
    UnmapViewOfFile(a);
    UnmapViewOfFile(b);
    CloseHandle(section);
}

static void image_step(void) {
    wchar_t path[MAX_PATH];
    UINT n = GetSystemDirectoryW(path, MAX_PATH);
    const wchar_t *name = L"\\kernel32.dll";
    HANDLE file, section;
    unsigned char *base;
    IMAGE_DOS_HEADER *dos;
    IMAGE_NT_HEADERS *nt;
    IMAGE_SECTION_HEADER *s;
    int i;
    while (*name) path[n++] = *name++;
    path[n] = 0;
    file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    result("open", file != INVALID_HANDLE_VALUE);
    if (file == INVALID_HANDLE_VALUE) return;
    section = CreateFileMappingW(file, NULL, PAGE_READONLY | SEC_IMAGE, 0, 0, NULL);
    result("image-section", section != NULL);
    CloseHandle(file);
    if (!section) return;
    base = (unsigned char *)MapViewOfFile(section, FILE_MAP_READ, 0, 0, 0);
    result("image-view", base != NULL);
    if (!base) return;
    dos = (IMAGE_DOS_HEADER *)base;
    nt = (IMAGE_NT_HEADERS *)(base + dos->e_lfanew);
    put("mz=");
    num(dos->e_magic == IMAGE_DOS_SIGNATURE);
    put(" pe=");
    num(nt->Signature == IMAGE_NT_SIGNATURE);
    nl();
    region("headers", base);
    s = IMAGE_FIRST_SECTION(nt);
    for (i = 0; i < nt->FileHeader.NumberOfSections; i++) {
        if ((s[i].Characteristics & IMAGE_SCN_MEM_EXECUTE) && s[i].Misc.VirtualSize) {
            region("code", base + s[i].VirtualAddress);
            break;
        }
    }
    result("unmap", UnmapViewOfFile(base));
    CloseHandle(section);
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
static unsigned char pattern(DWORD i) { return (unsigned char)((i * 7 + (i >> 12)) & 0xFF); }
/* Guest only: a 1 MiB file in %TEMP% written and read in one call each,
 * changed through a shared mapping and flushed, then deleted. */
static void files_step(void) {
    static wchar_t path[MAX_PATH];
    DWORD n = GetTempPathW(MAX_PATH, path), done = 0;
    const wchar_t *name = L"memprobe.bin";
    HANDLE file, section;
    unsigned char *buffer = (unsigned char *)VirtualAlloc(NULL, 0x100000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    unsigned char *view;
    DWORD i;
    EXPECT(buffer != NULL);
    if (!buffer) return;
    while (*name) path[n++] = *name++;
    path[n] = 0;
    for (i = 0; i < 0x100000; i++) buffer[i] = pattern(i);
    file = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    EXPECT(file != INVALID_HANDLE_VALUE);
    if (file == INVALID_HANDLE_VALUE) return;
    EXPECT(WriteFile(file, buffer, 0x100000, &done, NULL) && done == 0x100000);
    EXPECT(SetFilePointer(file, 0, NULL, FILE_BEGIN) == 0);
    for (i = 0; i < 0x100000; i++) buffer[i] = 0;
    EXPECT(ReadFile(file, buffer, 0x100000, &done, NULL) && done == 0x100000);
    for (i = 0; i < 0x100000; i += 4093) EXPECT(buffer[i] == pattern(i));
    section = CreateFileMappingW(file, NULL, PAGE_READWRITE, 0, 0, NULL);
    EXPECT(section != NULL);
    view = section ? (unsigned char *)MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0) : NULL;
    EXPECT(view != NULL);
    if (view) {
        EXPECT(view[0x54321] == pattern(0x54321));
        view[0x54321] = 0x5A;
        view[0xFFFFF] = 0xA5;
        EXPECT(FlushViewOfFile(view, 0));
        EXPECT(UnmapViewOfFile(view));
    }
    if (section) CloseHandle(section);
    EXPECT(SetFilePointer(file, 0, NULL, FILE_BEGIN) == 0);
    EXPECT(ReadFile(file, buffer, 0x100000, &done, NULL) && done == 0x100000);
    EXPECT(buffer[0x54321] == 0x5A && buffer[0xFFFFF] == 0xA5 && buffer[7] == 49);
    CloseHandle(file);
    EXPECT(DeleteFileW(path));
    VirtualFree(buffer, 0, MEM_RELEASE);
}

struct step {
    const char *name;
    void (*run)(void);
    int guest_only;
};
static const struct step steps[] = {{"sections", sections_step, 0},
                                    {"placeholders", placeholders_step, 0},
                                    {"copy", copy_step, 0},
                                    {"image", image_step, 0},
                                    {"files", files_step, 1}};
static const char *const expected[] = {
#include "memprobe.expected.h"
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
            put("MEM-A ");
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
            put("MEM-A ");
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
    put(failed ? "MEM-A FAILED\r\n" : "MEM-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

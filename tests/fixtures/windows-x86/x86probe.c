/* X86-A probe (P17 M9): a 32-bit (x86) Windows console program, MSVC, no C
 * runtime:
 *
 *   x86probe.exe raw | check
 *
 * Each step prints lines that must read the same on Windows 11 (WOW64)
 * and on Doors: console output, its own and kernel32's modules, heap and
 * virtual memory below 2 GB, implicit (the TLS template) and dynamic TLS,
 * a thread, x87 and SSE2 arithmetic with a varargs function, the WOW64
 * system queries, and structured exception handling through a frame the
 * program registers itself at FS:[0] (an access violation and
 * RaiseException, both continued by the handler). `raw` prints them
 * (x86probe.expected.h is generated from the owner's PC's output); `check`
 * compares. Linked /SAFESEH:NO: its handler is an ordinary function, as in
 * images built without SafeSEH. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <stdarg.h>
#include <emmintrin.h>
#include <intrin.h>

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
/* A wide string's last path component (or the folder before it),
 * lower-cased ASCII. */
static void wlower(const wchar_t *s, int count) {
    char b[2];
    int i;
    b[1] = 0;
    for (i = 0; i < count && s[i]; i++) {
        wchar_t c = s[i];
        if (c >= L'A' && c <= L'Z') c += 32;
        b[0] = c < 128 ? (char)c : '?';
        put(b);
    }
}
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
/* Where the last path component starts. Windows paths use '\'; a program
 * Doors staged in one of its own folders is named natively ("/system/bin/
 * x86probe.exe"), as for its x64 programs. */
static int last_separator(const wchar_t *s, int n) {
    while (n > 0 && s[n - 1] != L'\\' && s[n - 1] != L'/') n--;
    return n;
}

static void console_step(void) {
    put("hello from x86 pointer=");
    num((long)sizeof(void *));
    nl();
}
static void module_step(void) {
    wchar_t path[MAX_PATH];
    HMODULE self = GetModuleHandleW(NULL), k32 = GetModuleHandleW(L"kernel32.dll");
    DWORD n;
    int cut, folder;
    /* The image holding this code (where it lies is the loader's choice:
     * Windows' ASLR moves it). */
    {
        IMAGE_DOS_HEADER *dos = (IMAGE_DOS_HEADER *)self;
        IMAGE_NT_HEADERS32 *nt = (IMAGE_NT_HEADERS32 *)((char *)self + dos->e_lfanew);
        unsigned long code = (unsigned long)module_step, base = (unsigned long)self;
        put("self=");
        num(code >= base && code < base + nt->OptionalHeader.SizeOfImage);
        put(" granular=");
        num((base & 0xffff) == 0);
        put(" machine=");
        hex(nt->FileHeader.Machine);
    }
    put(" kernel32=");
    num(k32 != NULL);
    put(" below2g=");
    num((unsigned long)k32 < 0x80000000ul);
    nl();
    n = GetModuleFileNameW(NULL, path, MAX_PATH);
    cut = last_separator(path, (int)n);
    put("exe=");
    wlower(path + cut, (int)n - cut);
    nl();
    n = GetModuleFileNameW(k32, path, MAX_PATH);
    cut = last_separator(path, (int)n);
    folder = last_separator(path, cut - 1);
    put("kernel32 folder=");
    wlower(path + folder, cut - 1 - folder);
    put(" name=");
    wlower(path + cut, (int)n - cut);
    nl();
}
static void memory_step(void) {
    HANDLE heap = GetProcessHeap();
    unsigned char *a = (unsigned char *)HeapAlloc(heap, HEAP_ZERO_MEMORY, 100);
    unsigned char *v;
    put("heap=");
    num(a != NULL);
    put(" aligned=");
    num(((unsigned long)a & 7) == 0);
    put(" below2g=");
    num((unsigned long)a < 0x80000000ul);
    put(" zero=");
    num(a && a[0] == 0 && a[99] == 0);
    put(" free=");
    num(HeapFree(heap, 0, a));
    nl();
    v = (unsigned char *)VirtualAlloc(NULL, 65536, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    put("virtual=");
    num(v != NULL);
    put(" granular=");
    num(((unsigned long)v & 0xffff) == 0);
    put(" below2g=");
    num((unsigned long)v < 0x80000000ul);
    if (v) {
        v[65535] = 42;
        put(" last=");
        num(v[65535]);
        put(" free=");
        num(VirtualFree(v, 0, MEM_RELEASE));
    }
    nl();
}

/* Implicit TLS without the C runtime: the template between _tls_start and
 * _tls_end, the index the loader writes, and the directory. A variable of
 * the template is reached as compiled __declspec(thread) code reaches it:
 * the block array at FS:[0x2C], this image's _tls_index, the variable's
 * offset in the template (the C runtime's absolute __tls_array symbol,
 * which this program does not link, is that 0x2C). */
#pragma data_seg(".tls")
int _tls_start = 0;
#pragma data_seg(".tls$B")
int implicit_template = 5;
#pragma data_seg(".tls$ZZZ")
int _tls_end = 0;
#pragma data_seg()
ULONG _tls_index;
const IMAGE_TLS_DIRECTORY32 _tls_used = {(DWORD)&_tls_start, (DWORD)&_tls_end, (DWORD)&_tls_index, 0, 0, 0};
static int *implicit(void) {
    char **blocks = (char **)__readfsdword(0x2c);
    return (int *)(blocks[_tls_index] + ((char *)&implicit_template - (char *)&_tls_start));
}
#define implicit_value (*implicit())
int _fltused = 0;
static DWORD dynamic_index;

static DWORD WINAPI worker(LPVOID argument) {
    int first = implicit_value;
    implicit_value = 9;
    TlsSetValue(dynamic_index, (LPVOID)77);
    return (DWORD)argument * 2 + (DWORD)first * 1000 + (DWORD)TlsGetValue(dynamic_index) * 100000;
}
static void tls_step(void) {
    put("implicit=");
    num(implicit_value);
    implicit_value = 7;
    put(" after=");
    num(implicit_value);
    dynamic_index = TlsAlloc();
    put(" dynamic=");
    num(dynamic_index != TLS_OUT_OF_INDEXES);
    TlsSetValue(dynamic_index, (LPVOID)1234);
    put(" value=");
    num((long)TlsGetValue(dynamic_index));
    nl();
}
static void thread_step(void) {
    DWORD id = 0, code = 0;
    HANDLE thread = CreateThread(NULL, 0, worker, (LPVOID)21, 0, &id);
    put("thread=");
    num(thread != NULL);
    put(" id=");
    num(id != 0);
    put(" wait=");
    num((long)WaitForSingleObject(thread, 10000));
    GetExitCodeThread(thread, &code);
    /* 21 * 2, the worker's fresh implicit value 5, its own dynamic 77. */
    put(" code=");
    num((long)code);
    CloseHandle(thread);
    /* This thread's copies are untouched by the worker. */
    put(" mine=");
    num(implicit_value);
    put(" dynamic=");
    num((long)TlsGetValue(dynamic_index));
    nl();
    TlsFree(dynamic_index);
}
static double sum(int count, ...) {
    va_list list;
    double total = 0;
    int i;
    va_start(list, count);
    for (i = 0; i < count; i++) total += va_arg(list, double);
    va_end(list);
    return total;
}
static void math_step(void) {
    double x87_root, sse_root, total = sum(4, 1.5, 2.25, -0.75, 10.0);
    __m128d v = _mm_set_sd(2.0);
    __asm {
        fld qword ptr [total]
        fsqrt
        fstp qword ptr [x87_root]
    }
    sse_root = _mm_cvtsd_f64(_mm_sqrt_sd(v, v));
    put("varargs=");
    num((long)(total * 1000));
    put(" x87=");
    num((long)(x87_root * 1000000));
    put(" sse2=");
    num((long)(sse_root * 1000000));
    nl();
}
typedef BOOL(WINAPI *IsWow64Process2Function)(HANDLE, USHORT *, USHORT *);
static void system_step(void) {
    SYSTEM_INFO info;
    BOOL wow = FALSE;
    wchar_t path[MAX_PATH];
    UINT n;
    int i;
    for (i = 0; i < 2; i++) {
        memset(&info, 0, sizeof info);
        if (i == 0) {
            GetSystemInfo(&info);
            put("system");
        } else {
            GetNativeSystemInfo(&info);
            put("native");
        }
        put(" arch=");
        num(info.wProcessorArchitecture);
        put(" page=");
        num((long)info.dwPageSize);
        put(" min=");
        hex((unsigned long)info.lpMinimumApplicationAddress);
        put(" max=");
        hex((unsigned long)info.lpMaximumApplicationAddress);
        put(" type=");
        num((long)info.dwProcessorType);
        put(" granularity=");
        hex(info.dwAllocationGranularity);
        put(" cpus=");
        num(info.dwNumberOfProcessors > 0);
        nl();
    }
    put("wow64=");
    num(IsWow64Process(GetCurrentProcess(), &wow));
    put(" ");
    num(wow);
    {
        IsWow64Process2Function f =
            (IsWow64Process2Function)GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "IsWow64Process2");
        USHORT process = 0, native = 0;
        put(" process2=");
        num(f != NULL && f(GetCurrentProcess(), &process, &native));
        put(" machine=");
        hex(process);
        put(" native=");
        hex(native);
    }
    nl();
    n = GetSystemWow64DirectoryW(path, MAX_PATH);
    put("wow64dir=");
    wlower(path + last_separator(path, (int)n), (int)n - last_separator(path, (int)n));
    put(" length-ok=");
    num(n == (UINT)wlen(path));
    n = GetSystemDirectoryW(path, MAX_PATH);
    put(" systemdir=");
    wlower(path + last_separator(path, (int)n), (int)n - last_separator(path, (int)n));
    nl();
}

/* SEH: a frame registered at FS:[0] whose handler continues execution.
 * C4733: the handler is deliberately not a SafeSEH one (/SAFESEH:NO);
 * C4740: the inline assembly that registers it. */
#pragma warning(disable : 4733 4740)
static DWORD seen_code, seen_parameters, seen_first;
static DWORD recover_at;
static EXCEPTION_DISPOSITION __cdecl handler(EXCEPTION_RECORD *record, void *frame, CONTEXT *context, void *dispatcher) {
    (void)frame;
    (void)dispatcher;
    seen_code = record->ExceptionCode;
    seen_parameters = record->NumberParameters;
    seen_first = record->NumberParameters ? (DWORD)record->ExceptionInformation[0] : 0;
    if (record->ExceptionCode == EXCEPTION_ACCESS_VIOLATION) context->Eip = recover_at;
    return ExceptionContinueExecution;
}
static void seh_step(void) {
    volatile int *nowhere = (volatile int *)0;
    int value = 0;
    ULONG_PTR arguments[2] = {0x1234, 0x5678};
    __asm {
        push offset handler
        push dword ptr fs:[0]
        mov dword ptr fs:[0], esp
        mov eax, offset recovered
        mov recover_at, eax
    }
    value = *nowhere;
    __asm {
    recovered:
    }
    put("av code=");
    hex(seen_code);
    put(" parameters=");
    num((long)seen_parameters);
    put(" read=");
    num((long)seen_first);
    seen_code = 0;
    RaiseException(0xe0000001, 0, 2, arguments);
    put(" raised=");
    hex(seen_code);
    put(" count=");
    num((long)seen_parameters);
    put(" first=");
    hex(seen_first);
    __asm {
        mov eax, dword ptr [esp]
        mov dword ptr fs:[0], eax
        add esp, 8
    }
    put(" value=");
    num(value);
    nl();
}

/* Unwinding, as every __try/__finally and __except relies on: two frames,
 * the inner one passes the exception on, the outer one unwinds to itself
 * with RtlUnwind (the inner handler runs again with EXCEPTION_UNWINDING and
 * its frame leaves the chain), then lets the program continue. */
static char order[8];
static int order_length;
static DWORD unwind_flags;
static void *outer_frame;
static void note(char c) {
    if (order_length < 7) order[order_length++] = c;
    order[order_length] = 0;
}
static EXCEPTION_DISPOSITION __cdecl inner_handler(EXCEPTION_RECORD *record, void *frame, CONTEXT *context, void *dispatcher) {
    (void)frame;
    (void)context;
    (void)dispatcher;
    if (record->ExceptionFlags & 2) {
        unwind_flags = record->ExceptionFlags;
        note('u');
    } else {
        note('I');
    }
    return ExceptionContinueSearch;
}
static EXCEPTION_DISPOSITION __cdecl outer_handler(EXCEPTION_RECORD *record, void *frame, CONTEXT *context, void *dispatcher) {
    (void)context;
    (void)dispatcher;
    if (record->ExceptionFlags & 2) {
        note('x');
        return ExceptionContinueSearch;
    }
    note('O');
    RtlUnwind(frame, NULL, record, NULL);
    note('R');
    return ExceptionContinueExecution;
}
static void unwind_step(void) {
    __asm {
        push offset outer_handler
        push dword ptr fs:[0]
        mov dword ptr fs:[0], esp
        mov outer_frame, esp
        push offset inner_handler
        push dword ptr fs:[0]
        mov dword ptr fs:[0], esp
    }
    RaiseException(0xe0000002, 0, 0, NULL);
    put("unwind order=");
    put(order);
    put(" flags=");
    hex(unwind_flags & 0xff);
    /* Windows' dispatcher puts back the chain head it saw when it called
     * the handler (the inner frame's address), so the head is not compared:
     * both frames go, through the outer one's link. */
    __asm {
        mov eax, outer_frame
        mov eax, dword ptr [eax]
        mov dword ptr fs:[0], eax
        add esp, 16
    }
    put(" continued=1");
    nl();
}

/* A private DLL beside the program (x86dll.dll), linked at the program's
 * own base: relocated, its DllMain runs on LoadLibrary. */
static void dll_step(void) {
    typedef void(__cdecl * Report)(LONG *);
    HMODULE m = LoadLibraryW(L"x86dll.dll");
    LONG seen[4] = {0, 0, 0, 0};
    int i;
    put("dll loaded=");
    num(m != NULL);
    if (m) {
        Report report = (Report)GetProcAddress(m, "x86dll_report");
        put(" export=");
        num(report != NULL);
        if (report) report(seen);
        put(" relocated=");
        num((DWORD)m != 0x400000 && ((DWORD)m & 0xffff) == 0);
        put(" seen=");
        for (i = 0; i < 4; i++) {
            num(seen[i]);
            if (i < 3) put(",");
        }
        put(" free=");
        num(FreeLibrary(m));
    }
    /* A data file: a handle marked as one (bit 0), no exports through it,
     * its resources (x86dll.rc's RCDATA 1, "Doors data") readable. */
    {
        HMODULE d = LoadLibraryExW(L"x86dll.dll", NULL, LOAD_LIBRARY_AS_DATAFILE);
        put(" datafile=");
        num(d != NULL && ((DWORD)d & 3) == 1);
        if (d) {
            HRSRC r = FindResourceW(d, MAKEINTRESOURCEW(1), MAKEINTRESOURCEW(10));
            const char *data = r ? (const char *)LockResource(LoadResource(d, r)) : NULL;
            const char *want = "Doors data";
            int same = data != NULL && SizeofResource(d, r) == 10;
            for (i = 0; same && i < 10; i++) same = data[i] == want[i];
            put(" proc=");
            num(GetProcAddress(d, "x86dll_report") == NULL);
            put(" resource=");
            num(same);
            put(" unload=");
            num(FreeLibrary(d));
        }
    }
    nl();
}

struct step {
    const char *name;
    void (*run)(void);
};
static const struct step steps[] = {
    {"console", console_step}, {"module", module_step}, {"memory", memory_step}, {"tls", tls_step},
    {"thread", thread_step},   {"math", math_step},     {"system", system_step}, {"seh", seh_step},
    {"unwind", unwind_step},   {"dll", dll_step},
};
static const char *const expected[] = {
#include "x86probe.expected.h"
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
    int raw = last_word_is(line, L"raw"), failed = 0, i;
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
            put("X86-A ");
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
    put(failed ? "X86-A FAILED\r\n" : "X86-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

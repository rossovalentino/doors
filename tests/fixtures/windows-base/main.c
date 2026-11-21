#include <windows.h>
#include <errno.h>
extern void exercise_waits(void);
extern void exercise_async_files(void);
extern void exercise_crt(void);
typedef DWORD (WINAPI *LIFECYCLE_STATE)(void);
static LIFECYCLE_STATE LifecycleState;
static HANDLE preload_ready, preload_go;
static DWORD WINAPI preload_worker(LPVOID unused) {
    HMODULE module;
    typedef DWORD (WINAPI *COMPILER_VALUE)(void);
    COMPILER_VALUE value;
    (void)unused;
    if (!SetEvent(preload_ready) || WaitForSingleObject(preload_go,5000)!=WAIT_OBJECT_0) return 201;
    module=GetModuleHandleW(L"lifecycle.dll");
    value=(COMPILER_VALUE)(ULONG_PTR)GetProcAddress(module,"CompilerValue");
    if (!module || !value || value()!=0x11223344) return 202;
    return 200;
}
static volatile LONG allow_attach = 1;
__declspec(dllexport) BOOL WINAPI AllowDllAttach(void) { return allow_attach != 0; }
static const char text[] = "WINDOWS_BASE_OK lifecycle=1 tls=1 threads=1 memory=1 files=1 exception=1\r\n";
static const char *volatile relocated_text = text;
static DWORD shared_index;
static DWORD main_id;
static unsigned __int64 process_peb;
static volatile LONG seen_ids[2];
static volatile LONG hold_ready;
static volatile unsigned char *shared_memory;
volatile unsigned __int64 probe_resume;
extern int ProbeReadOnly(void *memory);
extern void ClobberSimd(void);
static PVOID fault_target;
static PVOID observer_token;
static volatile LONG fault_order;
static LONG CALLBACK observe_fault(PEXCEPTION_POINTERS pointers) {
    if (!pointers || !pointers->ExceptionRecord || !pointers->ContextRecord) ExitProcess(150);
    InterlockedIncrement(&fault_order);
    Sleep(0);
    ClobberSimd();
    return EXCEPTION_CONTINUE_SEARCH;
}
static LONG CALLBACK handle_fault(PEXCEPTION_POINTERS pointers) {
    DWORD diagnostic_bytes;
    DWORD64 base = 0, frame = 0;
    DWORD64 original_sp = pointers->ContextRecord->Rsp;
    DWORD64 original_pc = pointers->ContextRecord->Rip;
    PVOID handler_data = NULL;
    PRUNTIME_FUNCTION function;
    if (fault_order != 1 || pointers->ExceptionRecord->ExceptionCode != EXCEPTION_ACCESS_VIOLATION
        || pointers->ExceptionRecord->NumberParameters != 2 || pointers->ExceptionRecord->ExceptionInformation[0] != 1
        || pointers->ExceptionRecord->ExceptionInformation[1] != (ULONG_PTR)fault_target
        || !probe_resume || (pointers->ContextRecord->ContextFlags & CONTEXT_CONTROL) != CONTEXT_CONTROL) ExitProcess(151);
    if (!RemoveVectoredExceptionHandler(observer_token)) ExitProcess(152);
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), "UNWIND_LOOKUP\r\n", 15, &diagnostic_bytes, NULL);
    function = RtlLookupFunctionEntry(original_pc, &base, NULL);
    if (!function || !base) ExitProcess(153);
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), "UNWIND_FRAME\r\n", 14, &diagnostic_bytes, NULL);
    if (RtlVirtualUnwind(UNW_FLAG_NHANDLER, base, original_pc, function, pointers->ContextRecord,
        &handler_data, &frame, NULL) != NULL || frame != original_sp
        || pointers->ContextRecord->Rsp != original_sp + 48 || !pointers->ContextRecord->Rip
        || pointers->ContextRecord->Rip == original_pc) ExitProcess(154);
    pointers->ContextRecord->Rsp = original_sp;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), "UNWIND_PASS\r\n", 13, &diagnostic_bytes, NULL);
    pointers->ContextRecord->Rip = probe_resume;
    InterlockedIncrement(&fault_order);
    return EXCEPTION_CONTINUE_EXECUTION;
}
static BOOL reopening(void) {
    const WCHAR *line = GetCommandLineW();
    const WCHAR match[] = L"--reopen";
    for (; *line; ++line) {
        DWORD index = 0;
        while (match[index] && line[index] && match[index] == line[index]) ++index;
        if (!match[index]) return TRUE;
    }
    return FALSE;
}

static void file_work(const unsigned char *data) {
    const WCHAR *path = L"p10-data-\x03bb.bin";
    unsigned char buffer[1024];
    DWORD actual = 0;
    DWORD i;
    HANDLE file;
    BOOL reopen = reopening();
    if (!reopen) {
        file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file != INVALID_HANDLE_VALUE || GetLastError() != ERROR_FILE_NOT_FOUND) ExitProcess(130);
        SetLastError(0x1234);
        file = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file == INVALID_HANDLE_VALUE || GetLastError() != ERROR_SUCCESS) ExitProcess(131);
        if (CreateFileW(path, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL) != INVALID_HANDLE_VALUE
            || GetLastError() != ERROR_SHARING_VIOLATION) ExitProcess(132);
        if (CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL) != INVALID_HANDLE_VALUE
            || GetLastError() != ERROR_FILE_EXISTS) ExitProcess(133);
        if (!WriteFile(file, data, 1024, &actual, NULL) || actual != 1024 || !FlushFileBuffers(file) || !CloseHandle(file)) ExitProcess(134);
    }
    SetLastError(0x1234);
    file = CreateFileW(L"P10-DATA-\x03bb.BIN", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE || GetLastError() != ERROR_ALREADY_EXISTS) ExitProcess(135);
    if (!ReadFile(file, buffer, sizeof(buffer), &actual, NULL) || actual != sizeof(buffer)) ExitProcess(136);
    if (WriteFile(file, "x", 1, &actual, NULL) || actual != 0 || GetLastError() != ERROR_ACCESS_DENIED) ExitProcess(140);
    for (i = 0; i < sizeof(buffer); ++i) if (buffer[i] != data[i]) ExitProcess(137);
    if (!ReadFile(file, buffer, 1, &actual, NULL) || actual != 0 || !CloseHandle(file)) ExitProcess(138);
    if (ReadFile(file, buffer, 1, &actual, NULL) || actual != 0 || GetLastError() != ERROR_INVALID_HANDLE) ExitProcess(139);
}
static DWORD WINAPI held_worker(LPVOID unused) {
    (void)unused;
    InterlockedExchange(&hold_ready, 1);
    for (;;) Sleep(0);
}

static DWORD WINAPI worker(LPVOID argument) {
    HMODULE module = LoadLibraryW(L"lifecycle.dll");
    DWORD value = (DWORD)(ULONG_PTR)argument;
    DWORD id = GetCurrentThreadId();
    DWORD i;
    exercise_crt();
    if (!module || !GetProcAddress(module,"CompilerValue") || !id || id == main_id || TlsGetValue(shared_index) != NULL) return 101;
    InterlockedExchange(&seen_ids[value - 1], (LONG)id);
    if (!TlsSetValue(shared_index, argument)) return 102;
    for (i = 0; i < 512; ++i) shared_memory[(value - 1) * 512 + i] = (unsigned char)(i + value);
    for (i = 0; i < 100; ++i) {
        SetLastError(0x2000 + value);
        errno = 0x6000 + (int)value;
        Sleep(0);
        if (GetLastError() != 0x2000 + value || errno != 0x6000 + (int)value || TlsGetValue(shared_index) != argument) return 103;
        if (!__readgsqword(0x30) || __readgsqword(0x60) != process_peb) return 104;
    }
    if (!FreeLibrary(module)) return 199;
    return 80 + value;
}

void entry(void) {
    HMODULE module, duplicate;
    DWORD count = 0;
    DWORD index;
    HANDLE threads[2];
    DWORD i;
    DWORD previous;
    unsigned char *memory;
    HANDLE existing_thread;
    preload_ready=CreateEventW(NULL,TRUE,FALSE,NULL);
    preload_go=CreateEventW(NULL,TRUE,FALSE,NULL);
    existing_thread=CreateThread(NULL,0,preload_worker,NULL,0,NULL);
    if (!preload_ready || !preload_go || !existing_thread || WaitForSingleObject(preload_ready,5000)!=WAIT_OBJECT_0) ExitProcess(203);
    if (GetModuleHandleW(L"lifecycle.dll") != NULL) ExitProcess(160);
    if (LoadLibraryW(L"door-missing-fixture.dll") != NULL || GetLastError() != ERROR_MOD_NOT_FOUND) ExitProcess(161);
    allow_attach = 0;
    if (LoadLibraryW(L"lifecycle.dll") != NULL || GetLastError() != ERROR_DLL_INIT_FAILED || GetModuleHandleW(L"lifecycle.dll") != NULL) ExitProcess(159);
    allow_attach = 1;
    module = LoadLibraryW(L"lifecycle.dll");
    if (!module || GetModuleHandleW(L"LIFECYCLE.DLL") != module) ExitProcess(162);
    LifecycleState = (LIFECYCLE_STATE)(ULONG_PTR)GetProcAddress(module, "LifecycleState");
    if (!LifecycleState || LifecycleState() != 1) ExitProcess(91);
    {
        DWORD code=0;
        if (!SetEvent(preload_go) || WaitForSingleObject(existing_thread,5000)!=WAIT_OBJECT_0 || !GetExitCodeThread(existing_thread,&code) || code!=200
            || !CloseHandle(existing_thread) || !CloseHandle(preload_ready) || !CloseHandle(preload_go)) ExitProcess(204);
    }
    duplicate = LoadLibraryW(L"LIFECYCLE.DLL");
    if (duplicate != module || !FreeLibrary(duplicate) || LifecycleState() != 1) ExitProcess(163);
    if (GetProcAddress(module, "DoorMissingExport") != NULL || GetLastError() != ERROR_PROC_NOT_FOUND) ExitProcess(164);
    if (!FreeLibrary(module) || GetModuleHandleW(L"lifecycle.dll") != NULL) ExitProcess(165);
    module = LoadLibraryW(L"lifecycle.dll");
    LifecycleState = (LIFECYCLE_STATE)(ULONG_PTR)GetProcAddress(module, "LifecycleState");
    if (!module || !LifecycleState || LifecycleState() != 1) ExitProcess(166);
    exercise_crt();
    {
        HMODULE api = LoadLibraryW(L"api-ms-win-core-synch-l1-2-0.dll");
        typedef VOID (WINAPI *SLEEP_FN)(DWORD);
        SLEEP_FN api_sleep;
        if (!api) ExitProcess(167);
        api_sleep=(SLEEP_FN)(ULONG_PTR)GetProcAddress(api,"Sleep");
        if (!api_sleep) ExitProcess(168);
        api_sleep(0);
        if (!FreeLibrary(api)) ExitProcess(169);
    }
    memory = VirtualAlloc(NULL, 4096, MEM_RESERVE, PAGE_NOACCESS);
    if (!memory || VirtualAlloc(memory, 4096, MEM_RESERVE, PAGE_READWRITE) != NULL) ExitProcess(111);
    if (VirtualAlloc(memory, 4096, MEM_COMMIT, PAGE_READWRITE) != memory) ExitProcess(112);
    for (i = 0; i < 4096; ++i) if (memory[i] != 0) ExitProcess(113);
    memory[0] = 0x55;
    if (VirtualAlloc(memory, 4096, MEM_COMMIT, PAGE_READWRITE) != memory || memory[0] != 0x55) ExitProcess(114);
    if (!VirtualProtect(memory, 4096, PAGE_READONLY, &previous) || previous != PAGE_READWRITE) ExitProcess(115);
    {
        PVOID handler;
        fault_target = memory;
        observer_token = AddVectoredExceptionHandler(1, observe_fault);
        handler = AddVectoredExceptionHandler(0, handle_fault);
        if (!observer_token || !handler) ExitProcess(153);
        if (!ProbeReadOnly(memory)) ExitProcess(155);
        if (fault_order != 2 || !RemoveVectoredExceptionHandler(handler) || memory[0] != 0x55) ExitProcess(154);
    }
    if (!VirtualProtect(memory, 4096, PAGE_READWRITE, &previous) || previous != PAGE_READONLY) ExitProcess(117);
    shared_memory = memory;
    index = TlsAlloc();
    if (index == TLS_OUT_OF_INDEXES) ExitProcess(93);
    SetLastError(0x1234);
    if (TlsGetValue(index) != NULL || GetLastError() != ERROR_SUCCESS) ExitProcess(94);
    SetLastError(0x5678);
    if (!TlsSetValue(index, (LPVOID)0x11223344) || GetLastError() != 0x5678) ExitProcess(95);
    if (TlsGetValue(index) != (LPVOID)0x11223344 || !TlsFree(index)) ExitProcess(96);
    index = TlsAlloc();
    if (index == TLS_OUT_OF_INDEXES || TlsGetValue(index) != NULL || !TlsFree(index)) ExitProcess(97);
    if (TlsSetValue(TLS_OUT_OF_INDEXES, NULL) || GetLastError() != ERROR_INVALID_PARAMETER) ExitProcess(98);
    shared_index = TlsAlloc();
    if (shared_index == TLS_OUT_OF_INDEXES || !TlsSetValue(shared_index, (LPVOID)0xabcdef)) ExitProcess(105);
    main_id = GetCurrentThreadId();
    process_peb = __readgsqword(0x60);
    for (i = 0; i < 2; ++i) {
        threads[i] = CreateThread(NULL, 0, worker, (LPVOID)(ULONG_PTR)(i + 1), 0, NULL);
        if (!threads[i]) ExitProcess(106);
    }
    for (i = 0; i < 2; ++i) {
        DWORD code = 0;
        if (WaitForSingleObject(threads[i], 5000) != WAIT_OBJECT_0 || !GetExitCodeThread(threads[i], &code)
            || code != 81 + i || !CloseHandle(threads[i])) ExitProcess(107);
        if (CloseHandle(threads[i]) || GetLastError() != ERROR_INVALID_HANDLE) ExitProcess(108);
    }
    if (!seen_ids[0] || !seen_ids[1] || seen_ids[0] == seen_ids[1]
        || errno != ENOMEM
        || TlsGetValue(shared_index) != (LPVOID)0xabcdef || !TlsFree(shared_index)
        || LifecycleState() != 1) ExitProcess(109);
    exercise_waits();
    for (i = 0; i < 1024; ++i) if (memory[i] != (unsigned char)(i % 512 + 1 + i / 512)) ExitProcess(118);
    file_work(memory);
    exercise_async_files();
    if (!VirtualFree(memory, 4096, MEM_DECOMMIT) || !VirtualFree(memory, 4096, MEM_DECOMMIT)) ExitProcess(119);
    if (VirtualAlloc(memory, 4096, MEM_COMMIT, PAGE_READWRITE) != memory) ExitProcess(120);
    for (i = 0; i < 4096; ++i) if (memory[i] != 0) ExitProcess(121);
    if (VirtualFree(memory, 1, MEM_RELEASE) || !VirtualFree(memory, 0, MEM_RELEASE)) ExitProcess(122);
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), relocated_text, sizeof(text) - 1, &count, NULL)
        || count != sizeof(text) - 1) ExitProcess(92);
    if (!CreateThread(NULL, 0, held_worker, NULL, 0, NULL)) ExitProcess(110);
    while (!hold_ready) Sleep(0);
    ExitProcess(74);
}

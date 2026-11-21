/* Doors OS P13 fixture F3 (wingui-hang.exe): hung-window containment.
 *
 * Ordinary Windows SDK C without a CRT, built with the pinned MSVC toolchain.
 * No Door headers, imports or conditional paths: the same executable bytes run
 * on the Windows reference and in Door.
 *
 * Modes (one exact argument):
 *   (none)      interactive: a top-level window that pumps normally until it
 *               receives the character h or H, then stops retrieving messages
 *               for good (its window procedure blocks). Esc or Close exit 0.
 *               Containment (a "not responding" ghost, or close and terminate)
 *               belongs to the system; the process never recovers by itself.
 *   --selftest  non-interactive: a second thread's visible top-level window
 *               stops pumping on command; IsHungAppWindow, SendMessageTimeoutW
 *               (SMTO_NORMAL and SMTO_ABORTIFHUNG), PostMessageW and
 *               InternalGetWindowText are measured before and after the
 *               five-second hung threshold; the window is released, then a
 *               destination that destroys itself inside a sent message and a
 *               window that rejects WM_CREATE are checked; exits 82
 *   --reject    the application rejects its own initialization: its main
 *               window's WM_NCCREATE returns FALSE, CreateWindowExW fails and
 *               the process exits 83 without showing a window
 * Results are deterministic lines written to standard output (when the process
 * has one) and appended to wingui-hang.results in the working directory. Every
 * message goes to this process's own windows; it never injects system input or
 * changes settings.
 */
#define UNICODE
#define _UNICODE
#define WINVER 0x0a00
#define _WIN32_WINNT 0x0a00
#include <windows.h>

#define EXIT_SELFTEST 82
#define EXIT_REJECT 83

#define WM_STOP (WM_APP + 1)
#define WM_PING (WM_APP + 2)
#define WM_PING_POSTED (WM_APP + 3)
#define WM_COUNT (WM_APP + 4)
#define WM_DESTROY_SELF (WM_APP + 5)

#define SENTINEL 0x0badu

static const WCHAR main_class[] = L"DoorsHang";
static const WCHAR target_class[] = L"DoorsHangTarget";
static const WCHAR reject_class[] = L"DoorsHangReject";
static const WCHAR main_title[] = L"Doors hang fixture";
static const WCHAR target_title[] = L"Doors hang target";

static HINSTANCE instance;
static HWND main_hwnd, target_hwnd;
static HANDLE ready_event, stopped_event, release_event, never_event;
static volatile LONG pings_sent, pings_posted;
static int reject_mode;
static BOOL stopping;

/* ---- output -------------------------------------------------------------- */

static HANDLE out_handle;
static HANDLE results_handle;
static char line[512];
static DWORD line_len;

static void zero(void *memory, SIZE_T count)
{
    volatile BYTE *bytes = (volatile BYTE *)memory;
    while (count--)
        *bytes++ = 0;
}

static BOOL weq(const WCHAR *a, const WCHAR *b)
{
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return *a == *b;
}

static void put_char(char c)
{
    if (line_len < sizeof(line))
        line[line_len++] = c;
}

static void put(const char *text)
{
    while (*text)
        put_char(*text++);
}

static void put_dec(DWORD value)
{
    char digits[10];
    int count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    while (count)
        put_char(digits[--count]);
}

static void write_all(HANDLE handle, DWORD count)
{
    DWORD written = 0;
    if (!handle || handle == INVALID_HANDLE_VALUE || !count)
        return;
    if (!WriteFile(handle, line, count, &written, NULL) || written != count)
        ExitProcess(90);
}

/* Each line is flushed so a process that later hangs or is terminated has
   already made its earlier results durable. */
static void flush_line(void)
{
    DWORD count = line_len;
    line_len = 0;
    write_all(out_handle, count);
    write_all(results_handle, count);
    if (results_handle && results_handle != INVALID_HANDLE_VALUE)
        FlushFileBuffers(results_handle);
}

static __declspec(noreturn) void finish(UINT code)
{
    put("HANG_EXIT code=");
    put_dec(code);
    put("\r\n");
    flush_line();
    ExitProcess(code);
}

static __declspec(noreturn) void fail(UINT code)
{
    DWORD error = GetLastError();
    line_len = 0;
    put("HANG_FAIL code=");
    put_dec(code);
    put(" error=");
    put_dec(error);
    put("\r\n");
    flush_line();
    ExitProcess(code);
}

static void probe(const char *name, DWORD value)
{
    put("HANG_PROBE name=");
    put(name);
    put(" value=");
    put_dec(value);
    put("\r\n");
    flush_line();
}

/* ---- window procedures ----------------------------------------------------- */

static void paint_text(HWND hwnd, const WCHAR *first, int first_len, const WCHAR *second, int second_len)
{
    PAINTSTRUCT paint;
    HDC dc = BeginPaint(hwnd, &paint);
    if (!dc)
        return;
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, RGB(0x10, 0x20, 0x80));
    TextOutW(dc, 16, 16, first, first_len);
    TextOutW(dc, 16, 40, second, second_len);
    EndPaint(hwnd, &paint);
}

static LRESULT CALLBACK main_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    switch (message) {
    case WM_PAINT:
        paint_text(hwnd, L"Press H to stop responding.", 27, L"Esc closes the window.", 22);
        return 0;
    case WM_CHAR:
        if (wparam == L'h' || wparam == L'H') {
            /* Stop retrieving messages for good: block inside the window
               procedure on an event that is never signaled. */
            put("HANG_STOP pumping=0\r\n");
            flush_line();
            stopping = TRUE;
            WaitForSingleObject(never_event, INFINITE);
            return 0;
        }
        if (wparam == 27)
            DestroyWindow(hwnd);
        return 0;
    case WM_CLOSE:
        put("HANG_CLOSE\r\n");
        flush_line();
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static LRESULT CALLBACK target_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    switch (message) {
    case WM_PAINT:
        paint_text(hwnd, target_title, 17, L"Stops pumping on command.", 25);
        return 0;
    case WM_STOP:
        SetEvent(stopped_event);
        if (WaitForSingleObject(release_event, 60000) != WAIT_OBJECT_0)
            return 0;
        return 0x51;
    case WM_PING:
        InterlockedIncrement(&pings_sent);
        return 0x66;
    case WM_PING_POSTED:
        InterlockedIncrement(&pings_posted);
        return 0;
    case WM_COUNT:
        return (LRESULT)pings_sent * 16 + pings_posted;
    case WM_DESTROY_SELF:
        DestroyWindow(hwnd);
        return 0x77;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static LRESULT CALLBACK reject_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == WM_NCCREATE && reject_mode == 1)
        return FALSE;
    if (message == WM_CREATE && reject_mode == 2)
        return -1;
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static void register_classes(void)
{
    WNDCLASSEXW window;
    zero(&window, sizeof(window));
    window.cbSize = sizeof(window);
    window.style = CS_HREDRAW | CS_VREDRAW;
    window.lpfnWndProc = main_proc;
    window.hInstance = instance;
    window.hCursor = LoadCursorW(NULL, IDC_ARROW);
    window.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    window.lpszClassName = main_class;
    if (!RegisterClassExW(&window))
        fail(100);
    window.lpfnWndProc = target_proc;
    window.lpszClassName = target_class;
    if (!RegisterClassExW(&window))
        fail(101);
    window.lpfnWndProc = reject_proc;
    window.lpszClassName = reject_class;
    if (!RegisterClassExW(&window))
        fail(102);
}

/* ---- self-test ----------------------------------------------------------------- */

static void pump_for(DWORD milliseconds)
{
    ULONGLONG deadline = GetTickCount64() + milliseconds;
    MSG message;
    while (GetTickCount64() < deadline) {
        while (PeekMessageW(&message, NULL, 0, 0, PM_REMOVE)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        MsgWaitForMultipleObjects(0, NULL, FALSE, 5, QS_ALLINPUT);
    }
}

static DWORD WINAPI target_main(LPVOID unused)
{
    MSG message;
    (void)unused;
    target_hwnd = CreateWindowExW(0, target_class, target_title, WS_OVERLAPPEDWINDOW, 200, 160, 420, 260, NULL, NULL,
                                  instance, NULL);
    if (target_hwnd) {
        ShowWindow(target_hwnd, SW_SHOWNOACTIVATE);
        UpdateWindow(target_hwnd);
    }
    SetEvent(ready_event);
    if (!target_hwnd)
        return 1;
    while (GetMessageW(&message, NULL, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return 3;
}

static ULONGLONG timed_send(UINT message, UINT flags, UINT timeout, DWORD_PTR *result, BOOL *ok, DWORD *error)
{
    ULONGLONG started = GetTickCount64();
    SetLastError(SENTINEL);
    *ok = SendMessageTimeoutW(target_hwnd, message, 0, 0, flags, timeout, result) != 0;
    *error = GetLastError();
    return GetTickCount64() - started;
}

static __declspec(noreturn) void selftest(void)
{
    HANDLE thread;
    DWORD id, code, error;
    DWORD_PTR result = 0;
    ULONGLONG elapsed;
    BOOL ok;
    WCHAR text[64];
    HWND rejected;

    ready_event = CreateEventW(NULL, TRUE, FALSE, NULL);
    stopped_event = CreateEventW(NULL, TRUE, FALSE, NULL);
    release_event = CreateEventW(NULL, TRUE, FALSE, NULL);
    thread = ready_event && stopped_event && release_event ? CreateThread(NULL, 0, target_main, NULL, 0, &id) : NULL;
    if (!thread || WaitForSingleObject(ready_event, 5000) != WAIT_OBJECT_0 || !target_hwnd)
        fail(110);
    pump_for(200);
    probe("is_hung_initial", IsHungAppWindow(target_hwnd) != 0);
    probe("visible", IsWindowVisible(target_hwnd) != 0);

    /* Command the target to stop pumping. */
    if (!PostMessageW(target_hwnd, WM_STOP, 0, 0) || WaitForSingleObject(stopped_event, 5000) != WAIT_OBJECT_0)
        fail(111);
    probe("is_hung_stopped", IsHungAppWindow(target_hwnd) != 0);
    elapsed = timed_send(WM_PING, SMTO_NORMAL, 300, &result, &ok, &error);
    probe("send_normal_result", ok);
    probe("send_normal_error", error);
    probe("send_normal_waited", elapsed >= 250 && elapsed < 2000);
    SetLastError(SENTINEL);
    probe("post_while_stopped", PostMessageW(target_hwnd, WM_PING_POSTED, 0, 0) != 0);
    probe("internal_text", (DWORD)InternalGetWindowText(target_hwnd, text, 64));
    if (!weq(text, target_title))
        fail(112);

    /* Past the five-second threshold without retrieving messages. */
    pump_for(5600);
    probe("is_hung_after", IsHungAppWindow(target_hwnd) != 0);
    elapsed = timed_send(WM_PING, SMTO_ABORTIFHUNG, 1000, &result, &ok, &error);
    probe("send_abort_result", ok);
    probe("send_abort_error", error);
    probe("send_abort_fast", elapsed < 500);
    elapsed = timed_send(WM_PING, SMTO_NORMAL, 300, &result, &ok, &error);
    probe("send_hung_normal_result", ok);
    probe("send_hung_normal_waited", elapsed >= 250 && elapsed < 2000);
    probe("is_window_hung", IsWindow(target_hwnd) != 0);

    /* Release: the posted message is retrieved; timed-out sends are recorded. */
    if (!SetEvent(release_event))
        fail(113);
    if (!SendMessageTimeoutW(target_hwnd, WM_COUNT, 0, 0, SMTO_BLOCK, 5000, &result))
        fail(114);
    probe("pings_after_release", (DWORD)result);
    pump_for(300);
    probe("is_hung_released", IsHungAppWindow(target_hwnd) != 0);

    /* A destination that destroys itself inside a sent message. */
    result = (DWORD_PTR)SendMessageW(target_hwnd, WM_DESTROY_SELF, 0, 0);
    probe("destroy_self_result", (DWORD)result);
    probe("destroyed_is_window", IsWindow(target_hwnd) != 0);
    elapsed = timed_send(WM_PING, SMTO_NORMAL, 300, &result, &ok, &error);
    probe("send_destroyed_result", ok);
    probe("send_destroyed_error", error);
    SetLastError(SENTINEL);
    probe("post_destroyed", PostMessageW(target_hwnd, WM_PING, 0, 0) != 0);
    probe("post_destroyed_error", GetLastError());
    if (WaitForSingleObject(thread, 5000) != WAIT_OBJECT_0 || !GetExitCodeThread(thread, &code) || code != 3)
        fail(115);
    CloseHandle(thread);

    /* A window that rejects WM_CREATE is never returned. */
    reject_mode = 2;
    SetLastError(SENTINEL);
    rejected = CreateWindowExW(0, reject_class, L"reject", WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, NULL, NULL, instance,
                               NULL);
    probe("reject_create", rejected != NULL);
    probe("reject_create_error", GetLastError());
    put("HANG_OK hung=1 abort=1 released=1 destroyed=1 rejected=1\r\n");
    flush_line();
    finish(EXIT_SELFTEST);
}

static BOOL CALLBACK count_window(HWND hwnd, LPARAM count)
{
    (void)hwnd;
    ++*(DWORD *)count;
    return TRUE;
}

static __declspec(noreturn) void reject(void)
{
    DWORD error, windows = 0;
    HWND hwnd;
    reject_mode = 1;
    SetLastError(SENTINEL);
    hwnd = CreateWindowExW(0, reject_class, main_title, WS_OVERLAPPEDWINDOW | WS_VISIBLE, 200, 160, 420, 260, NULL,
                           NULL, instance, NULL);
    error = GetLastError();
    EnumThreadWindows(GetCurrentThreadId(), count_window, (LPARAM)&windows);
    put("HANG_REJECT window=");
    put_dec(hwnd != NULL);
    put(" error=");
    put_dec(error);
    put(" thread_windows=");
    put_dec(windows);
    put("\r\n");
    flush_line();
    finish(EXIT_REJECT);
}

static __declspec(noreturn) void interactive(void)
{
    MSG message;
    int result;
    never_event = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!never_event)
        fail(120);
    main_hwnd = CreateWindowExW(0, main_class, main_title, WS_OVERLAPPEDWINDOW, 200, 160, 420, 260, NULL, NULL,
                                instance, NULL);
    if (!main_hwnd)
        fail(121);
    ShowWindow(main_hwnd, SW_SHOWNORMAL);
    UpdateWindow(main_hwnd);
    SetForegroundWindow(main_hwnd);
    put("HANG_READY window=1\r\n");
    flush_line();
    while ((result = GetMessageW(&message, NULL, 0, 0)) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    if (result < 0)
        fail(122);
    finish((UINT)message.wParam);
}

/* The single argument after the program name, or an empty string. */
static const WCHAR *argument(void)
{
    const WCHAR *text = GetCommandLineW();
    if (*text == L'"') {
        ++text;
        while (*text && *text != L'"')
            ++text;
        if (*text)
            ++text;
    } else {
        while (*text && *text != L' ' && *text != L'\t')
            ++text;
    }
    while (*text == L' ' || *text == L'\t')
        ++text;
    return text;
}

__declspec(noreturn) void entry(void)
{
    const WCHAR *mode = argument();
    instance = GetModuleHandleW(NULL);
    out_handle = GetStdHandle(STD_OUTPUT_HANDLE);
    results_handle = CreateFileW(L"wingui-hang.results", GENERIC_WRITE, FILE_SHARE_READ, NULL, OPEN_ALWAYS,
                                 FILE_ATTRIBUTE_NORMAL, NULL);
    if (results_handle == INVALID_HANDLE_VALUE
        || SetFilePointer(results_handle, 0, NULL, FILE_END) == INVALID_SET_FILE_POINTER)
        fail(91);
    if (!instance)
        fail(92);
    if (mode[0] && !weq(mode, L"--selftest") && !weq(mode, L"--reject"))
        fail(93);
    put("HANG_START mode=");
    put(!mode[0] ? "interactive" : weq(mode, L"--selftest") ? "selftest" : "reject");
    put("\r\n");
    flush_line();
    register_classes();
    if (weq(mode, L"--selftest"))
        selftest();
    if (weq(mode, L"--reject"))
        reject();
    interactive();
}

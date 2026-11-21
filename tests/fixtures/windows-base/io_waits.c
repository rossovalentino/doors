/* Ordinary Windows SDK fixture: native-backed events, APC delivery and owned
 * pending file reads. No Door imports, CRT or implementation model bindings. */
#include <windows.h>

static HANDLE ready_event;
static volatile LONG callback_count;
static volatile LONG callback_value;
static volatile LONG callback_thread;
static DWORD owner_thread;
static HANDLE query_file;
static HANDLE query_gate;
static OVERLAPPED *query_request;
static DWORD WINAPI concurrent_query(LPVOID unused) {
    DWORD transferred;
    (void)unused;
    if (WaitForSingleObject(query_gate, 5000) != WAIT_OBJECT_0
        || !GetOverlappedResult(query_file, query_request, &transferred, TRUE)
        || transferred != 1024) return 186;
    return 0;
}
static VOID CALLBACK worker_apc(ULONG_PTR value) {
    callback_thread = (LONG)GetCurrentThreadId();
    callback_value = (LONG)value;
    InterlockedIncrement(&callback_count);
}
static DWORD WINAPI alertable_worker(LPVOID unused) {
    DWORD outcome;
    (void)unused;
    if (!SetEvent(ready_event)) return 161;
    outcome = SleepEx(5000, TRUE);
    if (outcome != WAIT_IO_COMPLETION || callback_count != 1
        || callback_value != 0x3456 || callback_thread != (LONG)GetCurrentThreadId()
        || GetCurrentThreadId() == owner_thread) return 162;
    return 0x44;
}
static VOID CALLBACK ordered_apc(ULONG_PTR value) {
    if ((LONG)value != callback_count + 1 || GetCurrentThreadId() != owner_thread) ExitProcess(163);
    InterlockedIncrement(&callback_count);
}
void exercise_waits(void) {
    HANDLE events[2];
    HANDLE thread;
    DWORD code;
    owner_thread = GetCurrentThreadId();
    events[0] = CreateEventW(NULL, TRUE, FALSE, NULL);
    events[1] = CreateEventW(NULL, FALSE, TRUE, NULL);
    if (!events[0] || !events[1]) ExitProcess(164);
    if (WaitForSingleObject(events[0], 0) != WAIT_TIMEOUT
        || WaitForSingleObject(events[1], 0) != WAIT_OBJECT_0
        || WaitForSingleObject(events[1], 0) != WAIT_TIMEOUT) ExitProcess(165);
    if (!SetEvent(events[0]) || WaitForSingleObject(events[0], 0) != WAIT_OBJECT_0
        || WaitForSingleObject(events[0], 0) != WAIT_OBJECT_0 || !ResetEvent(events[0])) ExitProcess(166);
    if (!SetEvent(events[1]) || WaitForMultipleObjects(2, events, FALSE, 1000) != WAIT_OBJECT_0 + 1
        || !SetEvent(events[0]) || !SetEvent(events[1])
        || WaitForMultipleObjectsEx(2, events, FALSE, 0, FALSE) != WAIT_OBJECT_0) ExitProcess(167);
    ready_event = events[0];
    if (!ResetEvent(ready_event)) ExitProcess(168);
    thread = CreateThread(NULL, 0, alertable_worker, NULL, 0, NULL);
    if (!thread || WaitForSingleObject(ready_event, 5000) != WAIT_OBJECT_0
        || QueueUserAPC(worker_apc, thread, 0x3456) == 0
        || WaitForSingleObjectEx(thread, 5000, FALSE) != WAIT_OBJECT_0
        || !GetExitCodeThread(thread, &code) || code != 0x44 || !CloseHandle(thread)) ExitProcess(169);
    callback_count = 0;
    if (!QueueUserAPC(ordered_apc, GetCurrentThread(), 1)
        || !QueueUserAPC(ordered_apc, GetCurrentThread(), 2)) ExitProcess(170);
    Sleep(0);
    if (callback_count != 0 || SleepEx(0, TRUE) != WAIT_IO_COMPLETION
        || callback_count != 2 || SleepEx(0, TRUE) != 0) ExitProcess(171);
    if (!CloseHandle(events[0]) || !CloseHandle(events[1])) ExitProcess(172);
    if (SetEvent(events[0]) || GetLastError() != ERROR_INVALID_HANDLE) ExitProcess(173);
}
static BOOL begin_read(HANDLE file, void *data, DWORD length, OVERLAPPED *overlapped) {
    BOOL result = ReadFile(file, data, length, NULL, overlapped);
    if (!result && GetLastError() != ERROR_IO_PENDING) ExitProcess(174);
    return result;
}
void exercise_async_files(void) {
    HANDLE file;
    HANDLE event;
    OVERLAPPED overlapped = {0};
    unsigned char data[1024];
    DWORD bytes, i;
    BOOL cancelled, result;
    event = CreateEventW(NULL, TRUE, TRUE, NULL);
    file = CreateFileW(L"p10-data-\x03bb.bin", GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL, OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED, NULL);
    if (!event || file == INVALID_HANDLE_VALUE) ExitProcess(175);
    overlapped.hEvent = event;
    begin_read(file, data, sizeof(data), &overlapped);
    if (WaitForSingleObject(event, 5000) != WAIT_OBJECT_0
        || !GetOverlappedResult(file, &overlapped, &bytes, TRUE) || bytes != sizeof(data)) ExitProcess(176);
    for (i = 0; i < sizeof(data); ++i)
        if (data[i] != (unsigned char)(i % 512 + 1 + i / 512)) ExitProcess(177);
    /* Two observers share one terminal record. A private worker handle must
     * remain retained when the first observer joins/closes its owner handle. */
    query_gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!query_gate) ExitProcess(186);
    query_file = file;
    query_request = &overlapped;
    for (i = 0; i < 8; ++i) {
        HANDLE observers[2];
        DWORD index, code;
        if (!ResetEvent(query_gate)) ExitProcess(186);
        begin_read(file, data, sizeof(data), &overlapped);
        for (index = 0; index < 2; ++index) {
            observers[index] = CreateThread(NULL, 0, concurrent_query, NULL, 0, NULL);
            if (!observers[index]) ExitProcess(186);
        }
        if (!SetEvent(query_gate)) ExitProcess(186);
        for (index = 0; index < 2; ++index) {
            if (WaitForSingleObject(observers[index], 5000) != WAIT_OBJECT_0
                || !GetExitCodeThread(observers[index], &code) || code != 0
                || !CloseHandle(observers[index])) ExitProcess(186);
        }
    }
    if (!CloseHandle(query_gate)) ExitProcess(186);
    /* Cancellation is a race: completion may win. Either outcome must be one
     * terminal record, and a second query must preserve the same result. */
    begin_read(file, data, sizeof(data), &overlapped);
    cancelled = CancelIoEx(file, &overlapped);
    if (!cancelled && GetLastError() != ERROR_NOT_FOUND) ExitProcess(178);
    result = GetOverlappedResult(file, &overlapped, &bytes, TRUE);
    if ((!result && GetLastError() != ERROR_OPERATION_ABORTED)
        || (result && bytes != sizeof(data))) ExitProcess(179);
    {
        DWORD again;
        BOOL repeated = GetOverlappedResult(file, &overlapped, &again, FALSE);
        if (repeated != result || (result && again != bytes)
            || (!repeated && GetLastError() != ERROR_OPERATION_ABORTED)) ExitProcess(180);
    }
    /* Event authority remains retained after the caller closes its handle. */
    begin_read(file, data, sizeof(data), &overlapped);
    if (!CloseHandle(event)) ExitProcess(181);
    for (i = 0; i < 20000; ++i) {
        if (GetOverlappedResult(file, &overlapped, &bytes, FALSE)) break;
        if (GetLastError() != ERROR_IO_INCOMPLETE) ExitProcess(181);
        Sleep(0);
    }
    if (i == 20000 || bytes != sizeof(data)) ExitProcess(181);
    overlapped.hEvent = NULL;
    begin_read(file, data, sizeof(data), &overlapped);
    if (!CloseHandle(file)) ExitProcess(182);
    /* Close drains the in-flight buffer in Door; Windows may cancel or complete
     * before CloseHandle returns. A new request never reuses the old handle. */
    if (CancelIoEx(file, NULL) || GetLastError() != ERROR_INVALID_HANDLE) ExitProcess(183);
    for (i = 0; i < 20000 && ((volatile OVERLAPPED *)&overlapped)->Internal == (ULONG_PTR)0x103; ++i) Sleep(0);
    if (i == 20000) ExitProcess(185);
}

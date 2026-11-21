/* SCM-A service (P16 M7): an ordinary Windows service program (Windows
 * SDK, no C runtime). StartServiceCtrlDispatcher connects it to the
 * Service Control Manager; ServiceMain reports START_PENDING then RUNNING
 * (accepting STOP, PAUSE/CONTINUE and SHUTDOWN), handles each control with
 * the status Windows expects, writes its pause/continue count to
 * HKLM\SOFTWARE\Doors Probe\Service when it stops, and exits. It counts
 * its starts there ("Runs"), and its "Mode" value makes it misbehave:
 * 1 crashes half a second after it runs (the process ends without
 * reporting STOPPED), 2 hangs in its STOP handler for 40 seconds.
 *
 *   doorsvc.exe          the service (started by the SCM)
 *
 * Guest only: installing a service changes the machine. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    unsigned char *d = destination;
    while (count--) *d++ = (unsigned char)value;
    return destination;
}

static SERVICE_STATUS_HANDLE handle;
static SERVICE_STATUS status;
static HANDLE stopped;
static DWORD pauses;
/* The stop's dwWin32ExitCode: the result of saving what the service saw. */
static DWORD exit_code;
/* HKLM\SOFTWARE\Doors Probe\Service "Mode": 0 normal, 1 crash, 2 hang. */
static DWORD mode;
#define KEY L"SOFTWARE\\Doors Probe\\Service"

static void report(DWORD state, DWORD wait) {
    status.dwServiceType = SERVICE_WIN32_OWN_PROCESS;
    status.dwCurrentState = state;
    status.dwControlsAccepted = state == SERVICE_START_PENDING
                                    ? 0
                                    : SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PAUSE_CONTINUE | SERVICE_ACCEPT_SHUTDOWN;
    status.dwWin32ExitCode = state == SERVICE_STOPPED ? exit_code : NO_ERROR;
    status.dwWaitHint = wait;
    status.dwCheckPoint = wait ? status.dwCheckPoint + 1 : 0;
    SetServiceStatus(handle, &status);
}

static DWORD WINAPI control(DWORD code, DWORD type, LPVOID data, LPVOID context) {
    (void)type;
    (void)data;
    (void)context;
    switch (code) {
    case SERVICE_CONTROL_STOP:
    case SERVICE_CONTROL_SHUTDOWN:
        report(SERVICE_STOP_PENDING, 2000);
        /* A hung handler: the SCM gives up on it after 30 seconds. */
        if (mode == 2) Sleep(40000);
        SetEvent(stopped);
        return NO_ERROR;
    case SERVICE_CONTROL_PAUSE:
        pauses++;
        report(SERVICE_PAUSED, 0);
        return NO_ERROR;
    case SERVICE_CONTROL_CONTINUE:
        report(SERVICE_RUNNING, 0);
        return NO_ERROR;
    case SERVICE_CONTROL_INTERROGATE:
        return NO_ERROR;
    default:
        return ERROR_CALL_NOT_IMPLEMENTED;
    }
}

static void WINAPI service_main(DWORD argc, LPWSTR *argv) {
    /* StartService's arguments follow the service's name: recorded as
     * "ArgumentCount" and "Arguments" (argv[1..] joined by '|') when there
     * are any, so a later start without them keeps the record. */
    if (argc > 1) {
        WCHAR joined[128];
        DWORD n = 0;
        for (DWORD i = 1; i < argc; i++) {
            for (const WCHAR *c = argv[i]; *c && n < 120; c++) joined[n++] = *c;
            if (i + 1 < argc && n < 120) joined[n++] = L'|';
        }
        joined[n++] = 0;
        RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"ArgumentCount", REG_DWORD, &argc, sizeof argc);
        RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"Arguments", REG_SZ, joined, n * sizeof(WCHAR));
    }
    handle = RegisterServiceCtrlHandlerExW(L"DoorsProbe", control, NULL);
    if (!handle) return;
    stopped = CreateEventW(NULL, TRUE, FALSE, NULL);
    report(SERVICE_START_PENDING, 3000);
    DWORD size = sizeof mode, runs = 0;
    if (RegGetValueW(HKEY_LOCAL_MACHINE, KEY, L"Mode", RRF_RT_REG_DWORD, NULL, &mode, &size) != ERROR_SUCCESS) mode = 0;
    size = sizeof runs;
    RegGetValueW(HKEY_LOCAL_MACHINE, KEY, L"Runs", RRF_RT_REG_DWORD, NULL, &runs, &size);
    runs++;
    RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"Runs", REG_DWORD, &runs, sizeof runs);
    /* Who it runs as: LocalSystem (SYSTEM, its profile under
     * system32\config), whoever started it. */
    {
        WCHAR user[64], profile[MAX_PATH];
        DWORD n = 64, p;
        if (GetUserNameW(user, &n))
            RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"Account", REG_SZ, user, n * sizeof(WCHAR));
        p = GetEnvironmentVariableW(L"USERPROFILE", profile, MAX_PATH);
        if (p && p < MAX_PATH)
            RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"Profile", REG_SZ, profile, (p + 1) * sizeof(WCHAR));
    }
    /* Its token: the user SID is LocalSystem's (S-1-5-18) and the token
     * is elevated (the full token). */
    {
        HANDLE token;
        if (OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
            BYTE buffer[128];
            TOKEN_ELEVATION elevation;
            DWORD n, system = 0, elevated = 0;
            if (GetTokenInformation(token, TokenUser, buffer, sizeof buffer, &n))
                system = IsWellKnownSid(((TOKEN_USER *)buffer)->User.Sid, WinLocalSystemSid) ? 1 : 0;
            if (GetTokenInformation(token, TokenElevation, &elevation, sizeof elevation, &n))
                elevated = elevation.TokenIsElevated ? 1 : 0;
            RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"LocalSystem", REG_DWORD, &system, sizeof system);
            RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, L"Elevated", REG_DWORD, &elevated, sizeof elevated);
            CloseHandle(token);
        }
    }
    report(SERVICE_RUNNING, 0);
    if (mode == 1) {
        /* A crash: the process ends while the service runs. */
        Sleep(500);
        ExitProcess(3);
    }
    WaitForSingleObject(stopped, INFINITE);
    /* What the service saw, for the probe to read back. */
    HKEY key;
    LSTATUS saved = RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Doors Probe\\Service", 0, NULL, 0, KEY_SET_VALUE,
                                    NULL, &key, NULL);
    /* Which call failed: 100000 + RegCreateKeyEx's error, else 200000 +
     * RegSetValueEx's. */
    DWORD failed_call = saved == ERROR_SUCCESS ? 0 : 100000;
    if (saved == ERROR_SUCCESS) {
        saved = RegSetValueExW(key, L"Pauses", 0, REG_DWORD, (const BYTE *)&pauses, sizeof pauses);
        if (saved != ERROR_SUCCESS) failed_call = 200000;
        RegCloseKey(key);
    }
    exit_code = saved == ERROR_SUCCESS ? 0 : failed_call + (DWORD)saved;
    report(SERVICE_STOPPED, 0);
}

void entry(void) {
    SERVICE_TABLE_ENTRYW table[] = {{L"DoorsProbe", service_main}, {NULL, NULL}};
    /* Not started by the SCM: ERROR_FAILED_SERVICE_CONTROLLER_CONNECT. */
    if (!StartServiceCtrlDispatcherW(table)) ExitProcess(GetLastError());
    ExitProcess(0);
}

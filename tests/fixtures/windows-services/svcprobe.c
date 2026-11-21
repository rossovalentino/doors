/* SCM-A probe (P16 M7): an ordinary Windows console program (Windows SDK,
 * no C runtime) that manages a service through advapi32's Service Control
 * Manager API: it installs doorsvc.exe (next to itself) as "DoorsProbe",
 * configures, starts, pauses, continues, enumerates and stops it, gives
 * it failure actions and lets it crash until they run out, runs a recovery
 * program after a crash, hangs its stop handler past the manager's 30
 * seconds, and deletes it, reading each state back.
 *
 *   svcprobe.exe [image]  the checks below (needs an elevated token);
 *                         image: the service program (default: doorsvc.exe
 *                         next to the probe)
 *   svcprobe.exe denied   what a standard (unelevated) user is refused
 *                         (SCM-D)
 *   svcprobe.exe reboot   a crashing service whose failure action restarts
 *                         the computer (SCM-R; the machine restarts)
 *   svcprobe.exe mark N   the recovery program: records N (the failure
 *                         count) in the probe's registry key
 *
 * Guest only: installing a service changes the machine.
 * One "SCM-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <shellapi.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    unsigned char *d = destination;
    while (count--) *d++ = (unsigned char)value;
    return destination;
}

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
    int negative = v < 0;
    unsigned long u = negative ? (unsigned long)-v : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (negative) b[--i] = '-';
    put(b + i);
}
static int failed_step;
static const char *tag = "SCM-A";
static void step(int id, const char *name, int ok, long detail) {
    put(tag);
    put(" ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static DWORD state(SC_HANDLE service, DWORD *pid) {
    SERVICE_STATUS_PROCESS s;
    DWORD needed = 0;
    memset(&s, 0, sizeof s);
    if (!QueryServiceStatusEx(service, SC_STATUS_PROCESS_INFO, (BYTE *)&s, sizeof s, &needed)) return 0;
    if (pid) *pid = s.dwProcessId;
    return s.dwCurrentState;
}
/* Polls until the service reaches `wanted` (10 s at most). */
static DWORD wait_for(SC_HANDLE service, DWORD wanted) {
    DWORD now = 0;
    for (int i = 0; i < 100; i++) {
        now = state(service, NULL);
        if (now == wanted) break;
        Sleep(100);
    }
    return now;
}
static BYTE config_buffer[8192];
static BYTE enum_buffer[16384];
#define KEY L"SOFTWARE\\Doors Probe\\Service"
static DWORD registry_number(const wchar_t *name) {
    DWORD v = 0, size = sizeof v;
    if (RegGetValueW(HKEY_LOCAL_MACHINE, KEY, name, RRF_RT_REG_DWORD, NULL, &v, &size) != ERROR_SUCCESS) return 0;
    return v;
}
static void set_number(const wchar_t *name, DWORD v) {
    HKEY key;
    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, KEY, 0, NULL, 0, KEY_SET_VALUE, NULL, &key, NULL) == ERROR_SUCCESS)
        RegCloseKey(key);
    RegSetKeyValueW(HKEY_LOCAL_MACHINE, KEY, name, REG_DWORD, &v, sizeof v);
}
/* A text value the service recorded under KEY into `out` (empty if none). */
static void registry_text(const wchar_t *name, wchar_t *text, DWORD units) {
    DWORD size = units * sizeof(wchar_t);
    text[0] = 0;
    if (RegGetValueW(HKEY_LOCAL_MACHINE, KEY, name, RRF_RT_REG_SZ, NULL, text, &size) != ERROR_SUCCESS) text[0] = 0;
}
/* Whether `text` ends with `tail`, ASCII case aside. */
static int ends_with(const wchar_t *text, const wchar_t *tail) {
    int n = 0, m = 0, i;
    while (text[n]) n++;
    while (tail[m]) m++;
    if (n < m) return 0;
    for (i = 0; i < m; i++) {
        wchar_t a = text[n - m + i], b = tail[i];
        if (a >= L'A' && a <= L'Z') a = (wchar_t)(a + 32);
        if (b >= L'A' && b <= L'Z') b = (wchar_t)(b + 32);
        if (a != b) return 0;
    }
    return 1;
}
/* Enables or disables SeShutdownPrivilege in this process's token (an
 * administrator's token holds it disabled). */
static BOOL shutdown_privilege(BOOL enable) {
    HANDLE token;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &token)) return FALSE;
    TOKEN_PRIVILEGES p;
    p.PrivilegeCount = 1;
    p.Privileges[0].Attributes = enable ? SE_PRIVILEGE_ENABLED : 0;
    BOOL ok = LookupPrivilegeValueW(NULL, SE_SHUTDOWN_NAME, &p.Privileges[0].Luid) &&
              AdjustTokenPrivileges(token, FALSE, &p, sizeof p, NULL, NULL) && GetLastError() == ERROR_SUCCESS;
    CloseHandle(token);
    return ok;
}
/* A decimal argument. */
static DWORD parse(const wchar_t *s) {
    DWORD v = 0;
    while (*s >= L'0' && *s <= L'9') v = v * 10 + (DWORD)(*s++ - L'0');
    return v;
}
/* The command line a recovery program runs: this probe in "mark" mode,
 * quoted, with the failure count the manager puts for %1%. */
static wchar_t recovery_command[MAX_PATH + 32];
static void make_recovery_command(void) {
    DWORD n = 0;
    recovery_command[n++] = L'"';
    n += GetModuleFileNameW(NULL, recovery_command + n, MAX_PATH);
    const wchar_t *tail = L"\" mark %1%";
    for (int i = 0; tail[i]; i++) recovery_command[n++] = tail[i];
    recovery_command[n] = 0;
}
static void finish(void) {
    put(tag);
    put(failed_step ? " FAILED\r\n" : " PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

/* SCM-D: a standard user may look but not change. */
static void denied(void) {
    tag = "SCM-D";
    SC_HANDLE all = OpenSCManagerW(NULL, NULL, SC_MANAGER_ALL_ACCESS);
    DWORD all_error = all ? 0 : GetLastError();
    if (all) CloseServiceHandle(all);
    step(1, "manager-denied", all == NULL && all_error == ERROR_ACCESS_DENIED, (long)all_error);
    SC_HANDLE manager = OpenSCManagerW(NULL, NULL, SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE);
    DWORD bytes = 0, count = 0, resume = 0;
    BOOL listed = manager && EnumServicesStatusExW(manager, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL,
                                                   enum_buffer, sizeof enum_buffer, &bytes, &count, &resume, NULL);
    step(2, "manager-read", manager && listed && count >= 1, (long)count);
    SC_HANDLE stop = manager ? OpenServiceW(manager, L"DoorsAudio", SERVICE_STOP) : NULL;
    DWORD stop_error = stop ? 0 : GetLastError();
    if (stop) CloseServiceHandle(stop);
    SC_HANDLE query = manager ? OpenServiceW(manager, L"DoorsAudio", SERVICE_QUERY_STATUS) : NULL;
    DWORD running = query ? state(query, NULL) : 0;
    if (query) CloseServiceHandle(query);
    step(3, "service-denied", stop == NULL && stop_error == ERROR_ACCESS_DENIED && running == SERVICE_RUNNING,
         (long)stop_error);
    if (manager) CloseServiceHandle(manager);
    finish();
}

/* SCM-R: restarting the computer as a failure action (run last: the
 * machine restarts). doorsvc.exe next to the probe is installed again as
 * DoorsProbe in its crashing mode; restarting the computer is refused
 * until the probe enables its shutdown privilege, then configured with a
 * 3-second delay, and the service started. It crashes half a second later
 * and the manager restarts the computer (the test watches it boot again). */
static void reboot(void) {
    tag = "SCM-R";
    wchar_t image[MAX_PATH + 16];
    DWORD n = GetModuleFileNameW(NULL, image, MAX_PATH);
    while (n && image[n - 1] != L'\\' && image[n - 1] != L'/') n--;
    const wchar_t *file = L"doorsvc.exe";
    for (int i = 0; file[i]; i++) image[n++] = file[i];
    image[n] = 0;
    SC_HANDLE manager = OpenSCManagerW(NULL, NULL, SC_MANAGER_ALL_ACCESS);
    SC_HANDLE service = manager ? CreateServiceW(manager, L"DoorsProbe", L"Doors Probe Service", SERVICE_ALL_ACCESS,
                                                 SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START,
                                                 SERVICE_ERROR_NORMAL, image, NULL, NULL, NULL, NULL, NULL)
                                : NULL;
    step(1, "create", service != NULL, service ? 0 : (long)GetLastError());
    SC_ACTION actions[1] = {{SC_ACTION_REBOOT, 3000}};
    SERVICE_FAILURE_ACTIONSW failure = {86400, (LPWSTR)L"Doors Probe restarts the computer.", NULL, 1, actions};
    BOOL early = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &failure);
    DWORD early_error = early ? 0 : GetLastError();
    BOOL privileged = shutdown_privilege(TRUE);
    BOOL configured = privileged && service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &failure);
    shutdown_privilege(FALSE);
    step(2, "privilege", !early && early_error == ERROR_ACCESS_DENIED && privileged && configured,
         (long)early_error * 100 + (long)privileged * 10 + (long)configured);
    set_number(L"Mode", 1);
    set_number(L"Runs", 0);
    BOOL started = service && StartServiceW(service, 0, NULL);
    step(3, "start", started, started ? 0 : (long)GetLastError());
    if (service) CloseServiceHandle(service);
    if (manager) CloseServiceHandle(manager);
    finish();
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    {
        int c = 0;
        LPWSTR *a = CommandLineToArgvW(GetCommandLineW(), &c);
        if (a && c >= 2 && same(a[1], L"denied")) denied();
        /* As the service's recovery program: record the failure count. */
        if (a && c >= 3 && same(a[1], L"mark")) {
            set_number(L"Recovered", parse(a[2]));
            ExitProcess(0);
        }
        if (a && c >= 2 && same(a[1], L"reboot")) reboot();
        if (a) LocalFree(a);
    }
    /* The service program: the first argument, else doorsvc.exe next to
     * this probe, installed (copied) as an installer would into
     * C:\Program Files\Doors Probe. */
    wchar_t image[MAX_PATH + 16];
    int argc = 0;
    LPWSTR *argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    DWORD n = 0;
    int installed = 0;
    if (argv && argc >= 2 && wlen(argv[1]) < MAX_PATH) {
        for (const wchar_t *p = argv[1]; *p; p++) image[n++] = *p;
        image[n] = 0;
        step(1, "install", 1, 0);
    } else {
        wchar_t source[MAX_PATH + 16];
        n = GetModuleFileNameW(NULL, source, MAX_PATH);
        while (n && source[n - 1] != L'\\' && source[n - 1] != L'/') n--;
        const wchar_t *file = L"doorsvc.exe";
        for (int i = 0; file[i]; i++) source[n++] = file[i];
        source[n] = 0;
        const wchar_t *target = L"C:\\Program Files\\Doors Probe\\doorsvc.exe";
        n = 0;
        while (target[n]) {
            image[n] = target[n];
            n++;
        }
        image[n] = 0;
        CreateDirectoryW(L"C:\\Program Files\\Doors Probe", NULL);
        installed = CopyFileW(source, image, FALSE) != 0;
        step(1, "install", installed, installed ? 0 : (long)GetLastError());
    }
    if (argv) LocalFree(argv);

    /* 2: the manager, with every right (an elevated token). */
    SC_HANDLE manager = OpenSCManagerW(NULL, NULL, SC_MANAGER_ALL_ACCESS);
    step(2, "open-manager", manager != NULL, manager ? 0 : (long)GetLastError());
    if (!manager) {
        put("SCM-A FAILED\r\n");
        ExitProcess(1);
    }
    /* 3: create; a second CreateService of the name is refused. */
    SC_HANDLE service = CreateServiceW(manager, L"DoorsProbe", L"Doors Probe Service", SERVICE_ALL_ACCESS,
                                       SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL,
                                       image, NULL, NULL, NULL, NULL, NULL);
    DWORD create_error = service ? 0 : GetLastError();
    SC_HANDLE again = CreateServiceW(manager, L"DoorsProbe", L"x", SERVICE_ALL_ACCESS, SERVICE_WIN32_OWN_PROCESS,
                                     SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL, image, NULL, NULL, NULL, NULL,
                                     NULL);
    DWORD again_error = again ? 0 : GetLastError();
    if (again) CloseServiceHandle(again);
    step(3, "create", service && again_error == ERROR_SERVICE_EXISTS, (long)(create_error ? create_error : again_error));
    /* 4: a description, then the configuration read back. */
    SERVICE_DESCRIPTIONW description = {L"Doors SCM-A probe service"};
    BOOL described = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_DESCRIPTION, &description);
    DWORD needed = 0;
    QUERY_SERVICE_CONFIGW *config = (QUERY_SERVICE_CONFIGW *)config_buffer;
    BOOL read = service && QueryServiceConfigW(service, config, sizeof config_buffer, &needed);
    int config_ok = read && config->dwStartType == SERVICE_DEMAND_START &&
                    config->dwServiceType == SERVICE_WIN32_OWN_PROCESS && same(config->lpDisplayName, L"Doors Probe Service") &&
                    wlen(config->lpBinaryPathName) > 0;
    step(4, "configure", described && config_ok, read ? (long)config->dwStartType : (long)GetLastError());
    /* 5: start: running in its own process, with two arguments (the
     *    second empty) that ServiceMain gets after the service's name. */
    LPCWSTR start_arguments[2] = {L"alpha", L""};
    BOOL started = service && StartServiceW(service, 2, start_arguments);
    DWORD start_error = started ? 0 : GetLastError();
    DWORD pid = 0;
    DWORD running = service ? wait_for(service, SERVICE_RUNNING) : 0;
    state(service, &pid);
    BOOL twice = service && StartServiceW(service, 0, NULL);
    DWORD twice_error = twice ? 0 : GetLastError();
    /* What the service recorded before reporting RUNNING: it runs as
     * LocalSystem (SYSTEM, S-1-5-18, elevated, its profile under
     * system32\config), whoever started it, with the start arguments
     * after its name (argc 3: name, "alpha", ""). */
    wchar_t account[64], profile[MAX_PATH], arguments[128];
    registry_text(L"Account", account, 64);
    registry_text(L"Profile", profile, MAX_PATH);
    registry_text(L"Arguments", arguments, 128);
    DWORD system = registry_number(L"LocalSystem"), elevated = registry_number(L"Elevated"),
          recorded_argc = registry_number(L"ArgumentCount");
    int identity = same(account, L"SYSTEM") && system == 1 && elevated == 1 &&
                   ends_with(profile, L"\\system32\\config\\systemprofile") && recorded_argc == 3 && same(arguments, L"alpha|");
    put("SCM-A note identity account_system=");
    number((long)same(account, L"SYSTEM"));
    put(" localsystem_sid=");
    number((long)system);
    put(" elevated=");
    number((long)elevated);
    put(" systemprofile=");
    number((long)ends_with(profile, L"\\system32\\config\\systemprofile"));
    put(" argc=");
    number((long)recorded_argc);
    put(" arguments_ok=");
    number((long)same(arguments, L"alpha|"));
    put("\r\n");
    step(5, "start",
         started && running == SERVICE_RUNNING && pid != 0 && twice_error == ERROR_SERVICE_ALREADY_RUNNING && identity,
         (long)(start_error ? start_error : running));
    /* 6: pause and continue. */
    SERVICE_STATUS st;
    BOOL paused = service && ControlService(service, SERVICE_CONTROL_PAUSE, &st);
    DWORD paused_state = service ? wait_for(service, SERVICE_PAUSED) : 0;
    BOOL resumed = service && ControlService(service, SERVICE_CONTROL_CONTINUE, &st);
    DWORD resumed_state = service ? wait_for(service, SERVICE_RUNNING) : 0;
    step(6, "pause-continue", paused && paused_state == SERVICE_PAUSED && resumed && resumed_state == SERVICE_RUNNING,
         (long)paused_state * 10 + (long)resumed_state);
    /* 7: EnumServicesStatusEx lists it running, by name and display name. */
    DWORD bytes = 0, count = 0, resume = 0;
    BOOL listed = EnumServicesStatusExW(manager, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL, enum_buffer,
                                        sizeof enum_buffer, &bytes, &count, &resume, NULL);
    int found = 0;
    ENUM_SERVICE_STATUS_PROCESSW *e = (ENUM_SERVICE_STATUS_PROCESSW *)enum_buffer;
    for (DWORD i = 0; listed && i < count; i++) {
        if (same(e[i].lpServiceName, L"DoorsProbe") && same(e[i].lpDisplayName, L"Doors Probe Service") &&
            e[i].ServiceStatusProcess.dwCurrentState == SERVICE_RUNNING && e[i].ServiceStatusProcess.dwProcessId == pid)
            found = 1;
    }
    step(7, "enumerate", listed && found && count >= 2, (long)count);
    /* 8: stop; the service wrote what it saw before it ended. */
    BOOL stopping = service && ControlService(service, SERVICE_CONTROL_STOP, &st);
    DWORD stopped = service ? wait_for(service, SERVICE_STOPPED) : 0;
    BOOL stop_again = service && ControlService(service, SERVICE_CONTROL_STOP, &st);
    DWORD stop_error = stop_again ? 0 : GetLastError();
    DWORD pauses = 0, size = sizeof pauses;
    LSTATUS got = RegGetValueW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Doors Probe\\Service", L"Pauses", RRF_RT_REG_DWORD,
                               NULL, &pauses, &size);
    /* Detail: one digit per condition (1 holds, 0 fails), then the second
     * stop's error and the registry read's status. */
    step(8, "stop", stopping && stopped == SERVICE_STOPPED && stop_error == ERROR_SERVICE_NOT_ACTIVE &&
                        got == ERROR_SUCCESS && pauses == 1,
         (long)(stopping != 0) * 10000 + (long)(stopped == SERVICE_STOPPED) * 1000 +
             (long)(stop_error == ERROR_SERVICE_NOT_ACTIVE) * 100 + (long)(got == ERROR_SUCCESS) * 10 +
             (long)(pauses == 1));
    if (stop_error != ERROR_SERVICE_NOT_ACTIVE || got != ERROR_SUCCESS || pauses != 1) {
        put("SCM-A stop detail error ");
        number((long)stop_error);
        put(" registry ");
        number((long)got);
        put(" pauses ");
        number((long)pauses);
        /* The service's own result of saving it (its stop exit code). */
        SERVICE_STATUS last;
        if (service && QueryServiceStatus(service, &last)) {
            put(" service ");
            number((long)last.dwWin32ExitCode);
        }
        put("\r\n");
    }
    /* 9: failure actions. Restarting the computer needs the shutdown
     * privilege enabled, which this token holds disabled: refused, then
     * accepted once the probe enables it. A recovery program's command line
     * is kept. Then: restart after a second twice, then nothing; the count
     * starts over after a day; an empty command deletes the program. */
    SC_ACTION reboot[1] = {{SC_ACTION_REBOOT, 1000}};
    SERVICE_FAILURE_ACTIONSW rebooting = {86400, NULL, NULL, 1, reboot};
    BOOL reboots = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &rebooting);
    DWORD reboot_error = reboots ? 0 : GetLastError();
    BOOL privileged = shutdown_privilege(TRUE);
    BOOL reboots_privileged =
        privileged && service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &rebooting);
    BOOL unprivileged = shutdown_privilege(FALSE);
    SC_ACTION program[1] = {{SC_ACTION_RUN_COMMAND, 500}};
    SERVICE_FAILURE_ACTIONSW running_program = {86400, NULL, (LPWSTR)L"notify.exe /fail=%1%", 1, program};
    BOOL runs_program = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &running_program);
    SERVICE_FAILURE_ACTIONSW *program_back = (SERVICE_FAILURE_ACTIONSW *)config_buffer;
    BOOL got_program = service && QueryServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, config_buffer,
                                                       sizeof config_buffer, &needed);
    int program_same = got_program && program_back->cActions == 1 && program_back->lpsaActions &&
                       program_back->lpsaActions[0].Type == SC_ACTION_RUN_COMMAND &&
                       program_back->lpsaActions[0].Delay == 500 && program_back->lpCommand &&
                       same(program_back->lpCommand, L"notify.exe /fail=%1%");
    SC_ACTION actions[3] = {{SC_ACTION_RESTART, 1000}, {SC_ACTION_RESTART, 1000}, {SC_ACTION_NONE, 0}};
    SERVICE_FAILURE_ACTIONSW failure = {86400, NULL, (LPWSTR)L"", 3, actions};
    BOOL configured = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &failure);
    SERVICE_FAILURE_ACTIONS_FLAG flag = {TRUE};
    BOOL flagged = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, &flag);
    SERVICE_FAILURE_ACTIONSW *back = (SERVICE_FAILURE_ACTIONSW *)config_buffer;
    BOOL got_back = service && QueryServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, config_buffer,
                                                    sizeof config_buffer, &needed);
    int same_back = got_back && back->dwResetPeriod == 86400 && back->cActions == 3 && back->lpsaActions &&
                    back->lpsaActions[0].Type == SC_ACTION_RESTART && back->lpsaActions[1].Delay == 1000 &&
                    back->lpsaActions[2].Type == SC_ACTION_NONE && back->lpCommand == NULL;
    SERVICE_FAILURE_ACTIONS_FLAG flag_back = {FALSE};
    BOOL got_flag = service && QueryServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, (BYTE *)&flag_back,
                                                    sizeof flag_back, &needed);
    flag.fFailureActionsOnNonCrashFailures = FALSE;
    if (service) ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, &flag);
    step(9, "recovery", configured && !reboots && reboot_error == ERROR_ACCESS_DENIED && privileged &&
                            reboots_privileged && unprivileged && runs_program && program_same && flagged &&
                            same_back && got_flag && flag_back.fFailureActionsOnNonCrashFailures,
         (long)reboot_error * 1000 + (long)reboots_privileged * 100 + (long)program_same * 10 + (long)same_back);
    /* 10: a crash loop: the service crashes half a second after it runs;
     * the manager restarts it twice, a second after each crash, and
     * leaves it stopped after the third. */
    set_number(L"Mode", 1);
    set_number(L"Runs", 0);
    BOOL crash_started = service && StartServiceW(service, 0, NULL);
    DWORD runs = 0, crash_state = 0;
    for (int i = 0; i < 200; i++) {
        runs = registry_number(L"Runs");
        crash_state = service ? state(service, NULL) : 0;
        if (runs >= 3 && crash_state == SERVICE_STOPPED) break;
        Sleep(100);
    }
    /* No fourth start follows. */
    Sleep(2500);
    DWORD final_runs = registry_number(L"Runs");
    SERVICE_STATUS crashed;
    memset(&crashed, 0, sizeof crashed);
    BOOL crash_status = service && QueryServiceStatus(service, &crashed);
    step(10, "crash-loop", crash_started && final_runs == 3 && crash_status && crashed.dwCurrentState == SERVICE_STOPPED &&
                               crashed.dwWin32ExitCode == 3,
         (long)final_runs * 100 + (long)(crash_status ? crashed.dwWin32ExitCode : 99));
    /* 11: a recovery program: after the next crash the manager runs this
     * probe ("mark %1%") as the service's user half a second later, with
     * the failure count; the count starts over first (reset period 0), so
     * it is 1. The service itself is not restarted. */
    make_recovery_command();
    SERVICE_FAILURE_ACTIONSW recovering = {0, NULL, recovery_command, 1, program};
    BOOL recovery_set = service && ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &recovering);
    set_number(L"Recovered", 0);
    set_number(L"Runs", 0);
    BOOL recovery_started = service && StartServiceW(service, 0, NULL);
    DWORD recovered = 0;
    for (int i = 0; i < 100 && !recovered; i++) {
        recovered = registry_number(L"Recovered");
        Sleep(100);
    }
    Sleep(1500);
    DWORD recovery_runs = registry_number(L"Runs");
    DWORD recovery_state = service ? state(service, NULL) : 0;
    step(11, "recovery-program", recovery_set && recovery_started && recovered == 1 && recovery_runs == 1 &&
                                     recovery_state == SERVICE_STOPPED,
         (long)recovered * 1000 + (long)recovery_runs * 10 + (long)recovery_state);
    /* 12: a stop handler that hangs: ControlService gives up after the
     * manager's 30 seconds with ERROR_SERVICE_REQUEST_TIMEOUT (event
     * 7011); the service stops on its own once its handler returns. */
    SERVICE_FAILURE_ACTIONSW none = {0, NULL, NULL, 0, actions};
    if (service) ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &none);
    set_number(L"Mode", 2);
    BOOL hang_started = service && StartServiceW(service, 0, NULL);
    DWORD hang_running = service ? wait_for(service, SERVICE_RUNNING) : 0;
    DWORD before = GetTickCount();
    BOOL hang_stop = service && ControlService(service, SERVICE_CONTROL_STOP, &st);
    DWORD hang_error = hang_stop ? 0 : GetLastError();
    DWORD waited = GetTickCount() - before;
    DWORD pending = service ? state(service, NULL) : 0;
    DWORD hang_end = 0;
    for (int i = 0; i < 300 && hang_end != SERVICE_STOPPED; i++) {
        hang_end = service ? state(service, NULL) : 0;
        Sleep(100);
    }
    set_number(L"Mode", 0);
    step(12, "hung-stop", hang_started && hang_running == SERVICE_RUNNING && !hang_stop &&
                              hang_error == ERROR_SERVICE_REQUEST_TIMEOUT && waited >= 29000 &&
                              pending == SERVICE_STOP_PENDING && hang_end == SERVICE_STOPPED,
         (long)hang_error * 100000L + (long)(waited / 1000) * 100 + (long)pending * 10 + (long)hang_end);
    /* 13: delete; the name is gone. */
    BOOL deleted = service && DeleteService(service);
    DWORD delete_error = deleted ? 0 : GetLastError();
    if (!deleted) {
        put("SCM-A delete detail error ");
        number((long)delete_error);
        put("\r\n");
    }
    if (service) CloseServiceHandle(service);
    SC_HANDLE gone = OpenServiceW(manager, L"DoorsProbe", SERVICE_QUERY_STATUS);
    DWORD gone_error = gone ? 0 : GetLastError();
    if (gone) CloseServiceHandle(gone);
    RegDeleteTreeW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Doors Probe");
    step(13, "delete", deleted && gone_error == ERROR_SERVICE_DOES_NOT_EXIST, (long)gone_error);
    CloseServiceHandle(manager);
    if (installed) {
        DeleteFileW(image);
        RemoveDirectoryW(L"C:\\Program Files\\Doors Probe");
    }
    put(failed_step ? "SCM-A FAILED\r\n" : "SCM-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

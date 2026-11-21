/* TSCHED-A probe (P16 M7): the Task Scheduler 2.0 API through COM, from an
 * ordinary Windows console program (Windows SDK, no C runtime):
 *
 *   tschedprobe.exe raw | check | guest | exit7
 *
 * The first steps only read the scheduler and build definitions in memory
 * (ITaskService, the root ITaskFolder, ITaskDefinition with its
 * registration info, triggers, actions, settings and principal, the task
 * XML, RegisterTaskDefinition with TASK_VALIDATE_ONLY, IDispatch through the
 * type library): their output is compared with what Windows 11 printed
 * (`raw` on the owner's PC; `tschedprobe.expected.h`). `guest` also
 * registers a task, runs it (this program with `exit7`), disables and
 * deletes it, which only the guest may do. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define COBJMACROS
#include <windows.h>
#include <oleauto.h>
#include <taskschd.h>
#include <intrin.h>

int _fltused = 0;
#pragma function(memset, memcpy)
void *memset(void *d, int v, size_t n) {
    __stosb((unsigned char *)d, (unsigned char)v, n);
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    __movsb((unsigned char *)d, (const unsigned char *)s, n);
    return d;
}

static const GUID clsid_scheduler = {0x0f87369f, 0xa4e5, 0x4cfc, {0xbd, 0x3e, 0x73, 0xe6, 0x15, 0x45, 0x72, 0xdd}};
static const GUID iid_service = {0x2faba4c7, 0x4da9, 0x4013, {0x96, 0x97, 0x20, 0xcc, 0x3f, 0xd4, 0x0f, 0x85}};
static const GUID iid_time = {0xb45747e0, 0xeba7, 0x4276, {0x9f, 0x29, 0x85, 0xc5, 0xbb, 0x30, 0x00, 0x06}};
static const GUID iid_daily = {0x126c5cd8, 0xb288, 0x41d5, {0x8d, 0xbf, 0xe4, 0x91, 0x44, 0x6a, 0xdc, 0x5c}};
static const GUID iid_exec = {0x4c3d624d, 0xfd6b, 0x49a3, {0xb9, 0xb7, 0x09, 0xcb, 0x3c, 0xd3, 0xf0, 0x47}};
static const GUID iid_null = {0, 0, 0, {0, 0, 0, 0, 0, 0, 0, 0}};
static const GUID iid_dispatch = {0x00020400, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};

static HANDLE out;
static char captured[65536];
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
static void hex(unsigned long v) {
    char b[11];
    int i;
    b[0] = '0';
    b[1] = 'x';
    for (i = 0; i < 8; i++) b[2 + i] = "0123456789abcdef"[(v >> (28 - 4 * i)) & 15];
    b[10] = 0;
    put(b);
}
static void wput(const wchar_t *s) {
    char b[1024];
    int k;
    if (!s) {
        put("(null)");
        return;
    }
    k = WideCharToMultiByte(CP_UTF8, 0, s, -1, b, sizeof b, NULL, NULL);
    if (k > 0) put(b);
}
/* A BSTR property: its text, then frees it. */
static void bput(BSTR b) {
    if (!b) {
        put("(null)");
        return;
    }
    put("\"");
    wput(b);
    put("\"");
    SysFreeString(b);
}
static void hr(const char *name, HRESULT h) {
    put(" ");
    put(name);
    put("=");
    hex((unsigned long)h);
}
static void flag(const char *name, VARIANT_BOOL b) {
    put(" ");
    put(name);
    put(b == VARIANT_TRUE ? "=true" : b == VARIANT_FALSE ? "=false" : "=?");
}
static void field(const char *name, long long v) {
    put(" ");
    put(name);
    put("=");
    num(v);
}
static void text(const char *name, BSTR b) {
    put(" ");
    put(name);
    put("=");
    bput(b);
}
static void nl(void) {
    put("\r\n");
}
/* Multi-line text, one "| line" per line. */
static void lines(BSTR b) {
    const wchar_t *p = b;
    wchar_t line[512];
    if (!b) {
        put("| (null)\r\n");
        return;
    }
    while (*p) {
        int n = 0;
        while (*p && *p != L'\r' && *p != L'\n' && n < 511) line[n++] = *p++;
        line[n] = 0;
        put("| ");
        wput(line);
        nl();
        if (*p == L'\r') p++;
        if (*p == L'\n') p++;
    }
    SysFreeString(b);
}
static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static VARIANT empty(void) {
    VARIANT v;
    VariantInit(&v);
    return v;
}
static VARIANT index(LONG i) {
    VARIANT v;
    VariantInit(&v);
    V_VT(&v) = VT_I4;
    V_I4(&v) = i;
    return v;
}
static BSTR s(const wchar_t *t) {
    return SysAllocString(t);
}

/* ---- the compared steps ------------------------------------------------------------------ */
static ITaskService *service;
static ITaskFolder *root;
static ITaskDefinition *definition;

static void service_step(void) {
    VARIANT_BOOL connected = 0;
    ITaskFolder *f = (ITaskFolder *)1;
    BSTR path = s(L"\\");
    ULONG version = 0;
    HRESULT h = CoCreateInstance(&clsid_scheduler, NULL, CLSCTX_INPROC_SERVER, &iid_service, (void **)&service);
    put("create");
    hr("hr", h);
    nl();
    if (h != S_OK) return;
    ITaskService_get_Connected(service, &connected);
    put("before");
    flag("connected", connected);
    hr("getfolder", ITaskService_GetFolder(service, path, &f));
    put(f ? " folder=set" : " folder=null");
    hr("newtask", ITaskService_NewTask(service, 0, &definition));
    if (definition) {
        ITaskDefinition_Release(definition);
        definition = NULL;
    }
    nl();
    h = ITaskService_Connect(service, empty(), empty(), empty(), empty());
    ITaskService_get_Connected(service, &connected);
    ITaskService_get_HighestVersion(service, &version);
    put("connect");
    hr("hr", h);
    flag("connected", connected);
    put(" highest=");
    hex(version);
    nl();
    SysFreeString(path);
}
static void folder_step(void) {
    BSTR name = NULL, path = NULL;
    ITaskFolder *f = NULL;
    IRegisteredTask *t = NULL;
    ITaskFolderCollection *c = NULL;
    BSTR p = s(L"\\"), none = s(L"\\DoorsNoSuchFolder"), blank = s(L""), task = s(L"DoorsNoSuchTask");
    put("root");
    hr("hr", ITaskService_GetFolder(service, p, &root));
    ITaskFolder_get_Name(root, &name);
    ITaskFolder_get_Path(root, &path);
    text("name", name);
    text("path", path);
    nl();
    put("folders");
    hr("missing", ITaskService_GetFolder(service, none, &f));
    put(f ? " folder=set" : " folder=null");
    hr("blank", ITaskService_GetFolder(service, blank, &f));
    if (f) {
        ITaskFolder_get_Path(f, &path);
        text("blank-path", path);
        ITaskFolder_Release(f);
        f = NULL;
    }
    hr("null", ITaskService_GetFolder(service, NULL, &f));
    if (f) {
        ITaskFolder_Release(f);
        f = NULL;
    }
    hr("subfolders", ITaskFolder_GetFolders(root, 0, &c));
    if (c) ITaskFolderCollection_Release(c);
    nl();
    put("tasks");
    hr("gettask", ITaskFolder_GetTask(root, task, &t));
    put(t ? " task=set" : " task=null");
    hr("delete", ITaskFolder_DeleteTask(root, task, 0));
    hr("delete-flags", ITaskFolder_DeleteTask(root, task, 1));
    nl();
    SysFreeString(p);
    SysFreeString(none);
    SysFreeString(blank);
    SysFreeString(task);
}
static void definition_step(void) {
    IRegistrationInfo *r = NULL;
    ITaskSettings *st = NULL;
    IPrincipal *pr = NULL;
    ITriggerCollection *tc = NULL;
    IActionCollection *ac = NULL;
    BSTR b = NULL;
    VARIANT_BOOL v = 0;
    LONG n = -1;
    INT pri = -1;
    TASK_INSTANCES_POLICY mi = 0;
    TASK_COMPATIBILITY cm = 0;
    TASK_LOGON_TYPE lt = 0;
    TASK_RUNLEVEL_TYPE rl = 0;
    ITaskDefinition *d = NULL;
    put("newtask");
    hr("flags1", ITaskService_NewTask(service, 1, &d));
    put(d ? " def=set" : " def=null");
    if (d) ITaskDefinition_Release(d);
    hr("hr", ITaskService_NewTask(service, 0, &definition));
    nl();
    ITaskDefinition_get_RegistrationInfo(definition, &r);
    put("reginfo");
    IRegistrationInfo_get_Author(r, &b);
    text("author", b);
    IRegistrationInfo_get_Description(r, &b);
    text("description", b);
    IRegistrationInfo_get_Date(r, &b);
    text("date", b);
    IRegistrationInfo_get_URI(r, &b);
    text("uri", b);
    IRegistrationInfo_get_Version(r, &b);
    text("version", b);
    nl();
    IRegistrationInfo_Release(r);
    ITaskDefinition_get_Settings(definition, &st);
    put("settings");
    ITaskSettings_get_AllowDemandStart(st, &v);
    flag("demand", v);
    ITaskSettings_get_StopIfGoingOnBatteries(st, &v);
    flag("stopbattery", v);
    ITaskSettings_get_DisallowStartIfOnBatteries(st, &v);
    flag("nobattery", v);
    ITaskSettings_get_AllowHardTerminate(st, &v);
    flag("hardterminate", v);
    ITaskSettings_get_StartWhenAvailable(st, &v);
    flag("whenavailable", v);
    ITaskSettings_get_RunOnlyIfNetworkAvailable(st, &v);
    flag("network", v);
    ITaskSettings_get_Enabled(st, &v);
    flag("enabled", v);
    ITaskSettings_get_Hidden(st, &v);
    flag("hidden", v);
    ITaskSettings_get_RunOnlyIfIdle(st, &v);
    flag("idle", v);
    ITaskSettings_get_WakeToRun(st, &v);
    flag("wake", v);
    nl();
    put("settings");
    ITaskSettings_get_MultipleInstances(st, &mi);
    field("instances", mi);
    ITaskSettings_get_Priority(st, &pri);
    field("priority", pri);
    ITaskSettings_get_Compatibility(st, &cm);
    field("compatibility", cm);
    ITaskSettings_get_RestartCount(st, &pri);
    field("restarts", pri);
    ITaskSettings_get_ExecutionTimeLimit(st, &b);
    text("limit", b);
    ITaskSettings_get_RestartInterval(st, &b);
    text("restartinterval", b);
    ITaskSettings_get_DeleteExpiredTaskAfter(st, &b);
    text("deleteafter", b);
    nl();
    put("settings-put");
    hr("priority11", ITaskSettings_put_Priority(st, 11));
    hr("instances9", ITaskSettings_put_MultipleInstances(st, 9));
    ITaskSettings_get_MultipleInstances(st, &mi);
    field("instances", mi);
    hr("instances1", ITaskSettings_put_MultipleInstances(st, TASK_INSTANCES_QUEUE));
    hr("limit", ITaskSettings_put_ExecutionTimeLimit(st, b = s(L"PT1H")));
    SysFreeString(b);
    ITaskSettings_get_ExecutionTimeLimit(st, &b);
    text("now", b);
    nl();
    ITaskSettings_Release(st);
    ITaskDefinition_get_Principal(definition, &pr);
    put("principal");
    IPrincipal_get_Id(pr, &b);
    text("id", b);
    IPrincipal_get_UserId(pr, &b);
    text("user", b);
    IPrincipal_get_LogonType(pr, &lt);
    field("logon", lt);
    IPrincipal_get_RunLevel(pr, &rl);
    field("runlevel", rl);
    nl();
    IPrincipal_Release(pr);
    ITaskDefinition_get_Triggers(definition, &tc);
    ITaskDefinition_get_Actions(definition, &ac);
    put("collections");
    ITriggerCollection_get_Count(tc, &n);
    field("triggers", n);
    IActionCollection_get_Count(ac, &n);
    field("actions", n);
    nl();
    ITriggerCollection_Release(tc);
    IActionCollection_Release(ac);
    put("xml-empty\r\n");
    ITaskDefinition_get_XmlText(definition, &b);
    lines(b);
}
static void triggers_step(void) {
    ITriggerCollection *tc = NULL;
    ITrigger *t = NULL, *u = NULL;
    ITimeTrigger *tt = NULL;
    IDailyTrigger *dt = NULL;
    IRepetitionPattern *rp = NULL;
    TASK_TRIGGER_TYPE2 type = 0;
    BSTR b = NULL;
    VARIANT_BOOL v = 0;
    LONG n = 0;
    short si = 0;
    ITaskDefinition_get_Triggers(definition, &tc);
    put("create");
    hr("time", ITriggerCollection_Create(tc, TASK_TRIGGER_TIME, &t));
    ITrigger_get_Type(t, &type);
    field("type", type);
    hr("bad", ITriggerCollection_Create(tc, 99, &u));
    put(u ? " trigger=set" : " trigger=null");
    hr("qi-time", ITrigger_QueryInterface(t, &iid_time, (void **)&tt));
    hr("qi-daily", ITrigger_QueryInterface(t, &iid_daily, (void **)&dt));
    nl();
    put("item");
    hr("0", ITriggerCollection_get_Item(tc, 0, &u));
    hr("1", ITriggerCollection_get_Item(tc, 1, &u));
    if (u) {
        put(u == t ? " same" : " other");
        ITrigger_Release(u);
        u = NULL;
    }
    hr("2", ITriggerCollection_get_Item(tc, 2, &u));
    nl();
    put("time");
    ITrigger_get_StartBoundary(t, &b);
    text("start", b);
    ITrigger_get_EndBoundary(t, &b);
    text("end", b);
    ITrigger_get_Enabled(t, &v);
    flag("enabled", v);
    ITrigger_get_Id(t, &b);
    text("id", b);
    ITrigger_get_ExecutionTimeLimit(t, &b);
    text("limit", b);
    ITimeTrigger_get_RandomDelay(tt, &b);
    text("random", b);
    hr("putstart", ITrigger_put_StartBoundary(t, b = s(L"2030-01-02T03:04:05")));
    SysFreeString(b);
    ITrigger_get_StartBoundary(t, &b);
    text("now", b);
    hr("putid", ITrigger_put_Id(t, b = s(L"Probe")));
    SysFreeString(b);
    nl();
    put("repetition");
    hr("get", ITrigger_get_Repetition(t, &rp));
    IRepetitionPattern_get_Interval(rp, &b);
    text("interval", b);
    IRepetitionPattern_get_Duration(rp, &b);
    text("duration", b);
    IRepetitionPattern_get_StopAtDurationEnd(rp, &v);
    flag("stop", v);
    hr("put", IRepetitionPattern_put_Interval(rp, b = s(L"PT5M")));
    SysFreeString(b);
    IRepetitionPattern_get_Interval(rp, &b);
    text("now", b);
    nl();
    IRepetitionPattern_Release(rp);
    put("daily");
    hr("create", ITriggerCollection_Create(tc, TASK_TRIGGER_DAILY, &u));
    if (u) {
        hr("qi", ITrigger_QueryInterface(u, &iid_daily, (void **)&dt));
        IDailyTrigger_get_DaysInterval(dt, &si);
        field("days", si);
        hr("putdays0", IDailyTrigger_put_DaysInterval(dt, 0));
        IDailyTrigger_Release(dt);
        ITrigger_Release(u);
        u = NULL;
    }
    hr("logon", ITriggerCollection_Create(tc, TASK_TRIGGER_LOGON, &u));
    if (u) {
        ITrigger_get_Type(u, &type);
        field("type", type);
        ITrigger_Release(u);
        u = NULL;
    }
    ITriggerCollection_get_Count(tc, &n);
    field("count", n);
    hr("remove3", ITriggerCollection_Remove(tc, index(3)));
    hr("remove9", ITriggerCollection_Remove(tc, index(9)));
    hr("remove2", ITriggerCollection_Remove(tc, index(2)));
    ITriggerCollection_get_Count(tc, &n);
    field("count", n);
    nl();
    ITimeTrigger_Release(tt);
    ITrigger_Release(t);
    ITriggerCollection_Release(tc);
}
static void actions_step(void) {
    IActionCollection *ac = NULL;
    IAction *a = NULL, *u = NULL;
    IExecAction *e = NULL;
    TASK_ACTION_TYPE type = 0;
    BSTR b = NULL;
    LONG n = 0;
    IRegistrationInfo *r = NULL;
    ITaskDefinition_get_Actions(definition, &ac);
    put("create");
    hr("exec", IActionCollection_Create(ac, TASK_ACTION_EXEC, &a));
    IAction_get_Type(a, &type);
    field("type", type);
    hr("bad", IActionCollection_Create(ac, 99, &u));
    hr("qi", IAction_QueryInterface(a, &iid_exec, (void **)&e));
    IActionCollection_get_Context(ac, &b);
    text("context", b);
    nl();
    put("exec");
    IExecAction_get_Path(e, &b);
    text("path", b);
    IExecAction_get_Arguments(e, &b);
    text("arguments", b);
    IExecAction_get_WorkingDirectory(e, &b);
    text("directory", b);
    IAction_get_Id(a, &b);
    text("id", b);
    hr("putpath", IExecAction_put_Path(e, b = s(L"C:\\Windows\\System32\\cmd.exe")));
    SysFreeString(b);
    hr("putargs", IExecAction_put_Arguments(e, b = s(L"/c exit 7")));
    SysFreeString(b);
    IExecAction_get_Path(e, &b);
    text("now", b);
    IActionCollection_get_Count(ac, &n);
    field("count", n);
    nl();
    IExecAction_Release(e);
    IAction_Release(a);
    IActionCollection_Release(ac);
    ITaskDefinition_get_RegistrationInfo(definition, &r);
    IRegistrationInfo_put_Author(r, b = s(L"Doors Probe"));
    SysFreeString(b);
    IRegistrationInfo_put_Description(r, b = s(L"A probe task & its <XML>"));
    SysFreeString(b);
    IRegistrationInfo_Release(r);
}
static void xml_step(void) {
    BSTR b = NULL;
    ITriggerCollection *tc = NULL;
    IActionCollection *ac = NULL;
    ITaskDefinition_get_XmlText(definition, &b);
    {
        int cr = 0, lf = 0, n = 0;
        const wchar_t *p;
        for (p = b; p && *p; p++) {
            n++;
            if (*p == L'\r') cr++;
            if (*p == L'\n') lf++;
        }
        put("definition");
        field("chars", n);
        field("cr", cr);
        field("lf", lf);
        nl();
    }
    lines(b);
    ITaskDefinition_get_Triggers(definition, &tc);
    put("triggers\r\n");
    ITaskDefinition_get_Actions(definition, &ac);
    IActionCollection_get_XmlText(ac, &b);
    put("actions\r\n");
    lines(b);
    ITriggerCollection_Release(tc);
    IActionCollection_Release(ac);
}
static void validate_step(void) {
    IRegisteredTask *t = (IRegisteredTask *)1;
    ITaskDefinition *d = NULL;
    BSTR name = s(L"DoorsProbeTask"), bad = s(L"Doors|Probe"), b = NULL;
    VARIANT none = empty();
    HRESULT h;
    put("validate");
    h = ITaskFolder_RegisterTaskDefinition(root, name, definition, TASK_VALIDATE_ONLY, none, none,
                                           TASK_LOGON_INTERACTIVE_TOKEN, none, &t);
    hr("good", h);
    put(t ? " task=set" : " task=null");
    t = NULL;
    h = ITaskFolder_RegisterTaskDefinition(root, bad, definition, TASK_VALIDATE_ONLY, none, none,
                                           TASK_LOGON_INTERACTIVE_TOKEN, none, &t);
    hr("badname", h);
    ITaskService_NewTask(service, 0, &d);
    h = ITaskFolder_RegisterTaskDefinition(root, name, d, TASK_VALIDATE_ONLY, none, none,
                                           TASK_LOGON_INTERACTIVE_TOKEN, none, &t);
    hr("noaction", h);
    h = ITaskFolder_RegisterTaskDefinition(root, name, definition, 0x40, none, none, TASK_LOGON_INTERACTIVE_TOKEN,
                                           none, &t);
    hr("badflags", h);
    h = ITaskFolder_RegisterTaskDefinition(root, name, definition, TASK_VALIDATE_ONLY, none, none,
                                           99, none, &t);
    hr("badlogon", h);
    nl();
    /* A definition from XML (put_XmlText), validated. */
    put("fromxml");
    ITaskDefinition_get_XmlText(definition, &b);
    hr("put", ITaskDefinition_put_XmlText(d, b));
    SysFreeString(b);
    {
        ITriggerCollection *tc = NULL;
        LONG n = 0;
        ITaskDefinition_get_Triggers(d, &tc);
        ITriggerCollection_get_Count(tc, &n);
        field("triggers", n);
        ITriggerCollection_Release(tc);
    }
    h = ITaskFolder_RegisterTaskDefinition(root, name, d, TASK_VALIDATE_ONLY, none, none,
                                           TASK_LOGON_INTERACTIVE_TOKEN, none, &t);
    hr("validate", h);
    put(t ? " task=set" : " task=null");
    hr("garbage", ITaskDefinition_put_XmlText(d, b = s(L"<Task>")));
    SysFreeString(b);
    nl();
    ITaskDefinition_Release(d);
    SysFreeString(name);
    SysFreeString(bad);
}
/* Every property of every part, set, and the XML Windows writes for it
   (the reference for Doors' task XML), then that XML read back. */
static const GUID iid_weekly = {0x5038fc98, 0x82ff, 0x436d, {0x87, 0x28, 0xa5, 0x12, 0xa5, 0x7c, 0x9d, 0xc1}};
static const GUID iid_monthly = {0x97c45ef1, 0x6b02, 0x4a1a, {0x9c, 0x0e, 0x1e, 0xbf, 0xba, 0x15, 0x00, 0xac}};
static const GUID iid_monthlydow = {0x77d025a3, 0x90fa, 0x43aa, {0xb5, 0x2e, 0xcd, 0xa5, 0x49, 0x9b, 0x94, 0x6a}};
static const GUID iid_logon = {0x72dade38, 0xfae4, 0x4b3e, {0xba, 0xf4, 0x5d, 0x00, 0x9a, 0xf0, 0x2b, 0x1c}};
static const GUID iid_boot = {0x2a9c35da, 0xd357, 0x41f4, {0xbb, 0xc1, 0x20, 0x7a, 0xc1, 0xb1, 0xf3, 0xcb}};
static const GUID iid_registration = {0x4c8fec3a, 0xc218, 0x4e0c, {0xb2, 0x3d, 0x62, 0x90, 0x24, 0xdb, 0x91, 0xa2}};
static const GUID iid_session = {0x754da71b, 0x4385, 0x4475, {0x9d, 0xd9, 0x59, 0x82, 0x94, 0xfa, 0x36, 0x41}};
static const GUID iid_event = {0xd45b0167, 0x9653, 0x4eef, {0xb9, 0x4f, 0x07, 0x32, 0xca, 0x7a, 0xf2, 0x51}};
static const GUID iid_comhandler = {0x6d2fd252, 0x75c5, 0x4f66, {0x90, 0xba, 0x2a, 0x7d, 0x8c, 0xc3, 0x03, 0x9f}};
static const GUID iid_email = {0x10f62c64, 0x7e16, 0x4314, {0xa0, 0xc2, 0x0c, 0x36, 0x83, 0xf9, 0x9d, 0x40}};
static const GUID iid_message = {0x505e9e68, 0xaf89, 0x46b8, {0xa3, 0x0f, 0x56, 0x16, 0x2a, 0x83, 0xd5, 0x37}};
static BSTR full_xml;
#define SETB(call, text)  \
    do {                  \
        BSTR t_ = s(text); \
        call;             \
        SysFreeString(t_); \
    } while (0)
static void trigger_base(ITrigger *t, const wchar_t *id) {
    SETB(ITrigger_put_Id(t, t_), id);
    SETB(ITrigger_put_StartBoundary(t, t_), L"2030-01-02T03:04:05");
}
static void fullxml_step(void) {
    ITaskDefinition *d = NULL;
    IRegistrationInfo *r = NULL;
    ITriggerCollection *tc = NULL;
    IActionCollection *ac = NULL;
    ITaskSettings *st = NULL;
    IIdleSettings *idle = NULL;
    INetworkSettings *net = NULL;
    IPrincipal *pr = NULL;
    ITrigger *t = NULL;
    IAction *a = NULL;
    IRepetitionPattern *rp = NULL;
    void *x = NULL;
    BSTR b = NULL;
    ITaskService_NewTask(service, 0, &d);
    ITaskDefinition_get_RegistrationInfo(d, &r);
    SETB(IRegistrationInfo_put_Description(r, t_), L"Description");
    SETB(IRegistrationInfo_put_Author(r, t_), L"Author");
    SETB(IRegistrationInfo_put_Version(r, t_), L"1.0");
    SETB(IRegistrationInfo_put_Date(r, t_), L"2026-11-09T10:00:00");
    SETB(IRegistrationInfo_put_Documentation(r, t_), L"Documentation");
    SETB(IRegistrationInfo_put_URI(r, t_), L"\\Doors\\Uri");
    SETB(IRegistrationInfo_put_Source(r, t_), L"Source");
    {
        VARIANT sd;
        VariantInit(&sd);
        V_VT(&sd) = VT_BSTR;
        V_BSTR(&sd) = s(L"D:(A;;FA;;;BA)");
        IRegistrationInfo_put_SecurityDescriptor(r, sd);
        VariantClear(&sd);
    }
    IRegistrationInfo_Release(r);
    ITaskDefinition_get_Triggers(d, &tc);
    put("triggers");
    /* 1 time */
    hr("time", ITriggerCollection_Create(tc, TASK_TRIGGER_TIME, &t));
    trigger_base(t, L"T1");
    SETB(ITrigger_put_EndBoundary(t, t_), L"2031-01-01T00:00:00");
    SETB(ITrigger_put_ExecutionTimeLimit(t, t_), L"PT2H");
    ITrigger_put_Enabled(t, VARIANT_FALSE);
    ITrigger_get_Repetition(t, &rp);
    SETB(IRepetitionPattern_put_Interval(rp, t_), L"PT1H");
    SETB(IRepetitionPattern_put_Duration(rp, t_), L"P1D");
    IRepetitionPattern_put_StopAtDurationEnd(rp, VARIANT_TRUE);
    IRepetitionPattern_Release(rp);
    ITrigger_QueryInterface(t, &iid_time, &x);
    SETB(ITimeTrigger_put_RandomDelay((ITimeTrigger *)x, t_), L"PT30M");
    ITimeTrigger_Release((ITimeTrigger *)x);
    ITrigger_Release(t);
    /* 2 daily */
    hr("daily", ITriggerCollection_Create(tc, TASK_TRIGGER_DAILY, &t));
    trigger_base(t, L"T2");
    ITrigger_QueryInterface(t, &iid_daily, &x);
    IDailyTrigger_put_DaysInterval((IDailyTrigger *)x, 3);
    SETB(IDailyTrigger_put_RandomDelay((IDailyTrigger *)x, t_), L"PT1M");
    IDailyTrigger_Release((IDailyTrigger *)x);
    ITrigger_Release(t);
    /* 3 weekly */
    hr("weekly", ITriggerCollection_Create(tc, TASK_TRIGGER_WEEKLY, &t));
    trigger_base(t, L"T3");
    hr("qi", ITrigger_QueryInterface(t, &iid_weekly, &x));
    IWeeklyTrigger_put_DaysOfWeek((IWeeklyTrigger *)x, 0x22);
    IWeeklyTrigger_put_WeeksInterval((IWeeklyTrigger *)x, 2);
    IWeeklyTrigger_Release((IWeeklyTrigger *)x);
    ITrigger_Release(t);
    /* 4 monthly */
    hr("monthly", ITriggerCollection_Create(tc, TASK_TRIGGER_MONTHLY, &t));
    trigger_base(t, L"T4");
    hr("qi", ITrigger_QueryInterface(t, &iid_monthly, &x));
    IMonthlyTrigger_put_DaysOfMonth((IMonthlyTrigger *)x, 0x5);
    IMonthlyTrigger_put_MonthsOfYear((IMonthlyTrigger *)x, 0x3);
    IMonthlyTrigger_put_RunOnLastDayOfMonth((IMonthlyTrigger *)x, VARIANT_TRUE);
    IMonthlyTrigger_Release((IMonthlyTrigger *)x);
    ITrigger_Release(t);
    /* 5 monthly day of week */
    hr("monthlydow", ITriggerCollection_Create(tc, TASK_TRIGGER_MONTHLYDOW, &t));
    trigger_base(t, L"T5");
    hr("qi", ITrigger_QueryInterface(t, &iid_monthlydow, &x));
    IMonthlyDOWTrigger_put_DaysOfWeek((IMonthlyDOWTrigger *)x, 0x2);
    IMonthlyDOWTrigger_put_WeeksOfMonth((IMonthlyDOWTrigger *)x, 0x1);
    IMonthlyDOWTrigger_put_MonthsOfYear((IMonthlyDOWTrigger *)x, 0xFFF);
    IMonthlyDOWTrigger_put_RunOnLastWeekOfMonth((IMonthlyDOWTrigger *)x, VARIANT_TRUE);
    IMonthlyDOWTrigger_Release((IMonthlyDOWTrigger *)x);
    ITrigger_Release(t);
    /* 6 logon */
    hr("logon", ITriggerCollection_Create(tc, TASK_TRIGGER_LOGON, &t));
    SETB(ITrigger_put_Id(t, t_), L"T6");
    hr("qi", ITrigger_QueryInterface(t, &iid_logon, &x));
    SETB(ILogonTrigger_put_Delay((ILogonTrigger *)x, t_), L"PT15S");
    SETB(ILogonTrigger_put_UserId((ILogonTrigger *)x, t_), L"DOORS\\user");
    ILogonTrigger_Release((ILogonTrigger *)x);
    ITrigger_Release(t);
    /* 7 boot */
    hr("boot", ITriggerCollection_Create(tc, TASK_TRIGGER_BOOT, &t));
    hr("qi", ITrigger_QueryInterface(t, &iid_boot, &x));
    SETB(IBootTrigger_put_Delay((IBootTrigger *)x, t_), L"PT1M");
    IBootTrigger_Release((IBootTrigger *)x);
    ITrigger_Release(t);
    /* 8 idle */
    hr("idle", ITriggerCollection_Create(tc, TASK_TRIGGER_IDLE, &t));
    ITrigger_Release(t);
    /* 9 registration */
    hr("registration", ITriggerCollection_Create(tc, TASK_TRIGGER_REGISTRATION, &t));
    hr("qi", ITrigger_QueryInterface(t, &iid_registration, &x));
    SETB(IRegistrationTrigger_put_Delay((IRegistrationTrigger *)x, t_), L"PT5S");
    IRegistrationTrigger_Release((IRegistrationTrigger *)x);
    ITrigger_Release(t);
    /* 10 session state change */
    hr("session", ITriggerCollection_Create(tc, TASK_TRIGGER_SESSION_STATE_CHANGE, &t));
    hr("qi", ITrigger_QueryInterface(t, &iid_session, &x));
    ISessionStateChangeTrigger_put_StateChange((ISessionStateChangeTrigger *)x, TASK_SESSION_LOCK);
    SETB(ISessionStateChangeTrigger_put_Delay((ISessionStateChangeTrigger *)x, t_), L"PT1S");
    SETB(ISessionStateChangeTrigger_put_UserId((ISessionStateChangeTrigger *)x, t_), L"DOORS\\user");
    ISessionStateChangeTrigger_Release((ISessionStateChangeTrigger *)x);
    ITrigger_Release(t);
    /* 11 event */
    hr("event", ITriggerCollection_Create(tc, TASK_TRIGGER_EVENT, &t));
    hr("qi", ITrigger_QueryInterface(t, &iid_event, &x));
    SETB(IEventTrigger_put_Subscription((IEventTrigger *)x, t_),
         L"<QueryList><Query Id=\"0\" Path=\"System\"><Select Path=\"System\">*</Select></Query></QueryList>");
    SETB(IEventTrigger_put_Delay((IEventTrigger *)x, t_), L"PT10S");
    IEventTrigger_Release((IEventTrigger *)x);
    ITrigger_Release(t);
    nl();
    ITriggerCollection_Release(tc);
    ITaskDefinition_get_Actions(d, &ac);
    SETB(IActionCollection_put_Context(ac, t_), L"Author");
    put("actions");
    hr("exec", IActionCollection_Create(ac, TASK_ACTION_EXEC, &a));
    SETB(IAction_put_Id(a, t_), L"A1");
    IAction_QueryInterface(a, &iid_exec, &x);
    SETB(IExecAction_put_Path((IExecAction *)x, t_), L"C:\\Program Files\\Doors\\probe.exe");
    SETB(IExecAction_put_Arguments((IExecAction *)x, t_), L"-a \"b c\"");
    SETB(IExecAction_put_WorkingDirectory((IExecAction *)x, t_), L"C:\\Temp");
    IExecAction_Release((IExecAction *)x);
    IAction_Release(a);
    hr("com", IActionCollection_Create(ac, TASK_ACTION_COM_HANDLER, &a));
    if (a) {
        IAction_QueryInterface(a, &iid_comhandler, &x);
        SETB(IComHandlerAction_put_ClassId((IComHandlerAction *)x, t_), L"{0F87369F-A4E5-4CFC-BD3E-73E6154572DD}");
        SETB(IComHandlerAction_put_Data((IComHandlerAction *)x, t_), L"data");
        IComHandlerAction_Release((IComHandlerAction *)x);
        IAction_Release(a);
        a = NULL;
    }
    hr("email", IActionCollection_Create(ac, TASK_ACTION_SEND_EMAIL, &a));
    if (a) {
        IAction_QueryInterface(a, &iid_email, &x);
        SETB(IEmailAction_put_Server((IEmailAction *)x, t_), L"smtp.example.com");
        SETB(IEmailAction_put_Subject((IEmailAction *)x, t_), L"Subject");
        SETB(IEmailAction_put_To((IEmailAction *)x, t_), L"to@example.com");
        SETB(IEmailAction_put_From((IEmailAction *)x, t_), L"from@example.com");
        SETB(IEmailAction_put_Body((IEmailAction *)x, t_), L"Body");
        IEmailAction_Release((IEmailAction *)x);
        IAction_Release(a);
        a = NULL;
    }
    hr("message", IActionCollection_Create(ac, TASK_ACTION_SHOW_MESSAGE, &a));
    if (a) {
        IAction_QueryInterface(a, &iid_message, &x);
        SETB(IShowMessageAction_put_Title((IShowMessageAction *)x, t_), L"Title");
        SETB(IShowMessageAction_put_MessageBody((IShowMessageAction *)x, t_), L"Message");
        IShowMessageAction_Release((IShowMessageAction *)x);
        IAction_Release(a);
        a = NULL;
    }
    nl();
    IActionCollection_Release(ac);
    ITaskDefinition_get_Settings(d, &st);
    ITaskSettings_put_AllowDemandStart(st, VARIANT_FALSE);
    SETB(ITaskSettings_put_RestartInterval(st, t_), L"PT5M");
    ITaskSettings_put_RestartCount(st, 3);
    ITaskSettings_put_MultipleInstances(st, TASK_INSTANCES_PARALLEL);
    ITaskSettings_put_StopIfGoingOnBatteries(st, VARIANT_FALSE);
    ITaskSettings_put_DisallowStartIfOnBatteries(st, VARIANT_FALSE);
    ITaskSettings_put_AllowHardTerminate(st, VARIANT_FALSE);
    ITaskSettings_put_StartWhenAvailable(st, VARIANT_TRUE);
    ITaskSettings_put_RunOnlyIfNetworkAvailable(st, VARIANT_TRUE);
    SETB(ITaskSettings_put_ExecutionTimeLimit(st, t_), L"PT4H");
    ITaskSettings_put_Enabled(st, VARIANT_FALSE);
    SETB(ITaskSettings_put_DeleteExpiredTaskAfter(st, t_), L"PT1H");
    ITaskSettings_put_Priority(st, 4);
    ITaskSettings_put_Hidden(st, VARIANT_TRUE);
    ITaskSettings_put_RunOnlyIfIdle(st, VARIANT_TRUE);
    ITaskSettings_put_WakeToRun(st, VARIANT_TRUE);
    ITaskSettings_get_IdleSettings(st, &idle);
    SETB(IIdleSettings_put_IdleDuration(idle, t_), L"PT5M");
    SETB(IIdleSettings_put_WaitTimeout(idle, t_), L"PT30M");
    IIdleSettings_put_StopOnIdleEnd(idle, VARIANT_FALSE);
    IIdleSettings_put_RestartOnIdle(idle, VARIANT_TRUE);
    IIdleSettings_Release(idle);
    ITaskSettings_get_NetworkSettings(st, &net);
    SETB(INetworkSettings_put_Name(net, t_), L"Doors Network");
    SETB(INetworkSettings_put_Id(net, t_), L"{D0025D01-0000-0000-0000-000000000001}");
    INetworkSettings_Release(net);
    ITaskSettings_Release(st);
    ITaskDefinition_get_Principal(d, &pr);
    SETB(IPrincipal_put_Id(pr, t_), L"Author");
    SETB(IPrincipal_put_DisplayName(pr, t_), L"Display");
    SETB(IPrincipal_put_UserId(pr, t_), L"DOORS\\user");
    IPrincipal_put_LogonType(pr, TASK_LOGON_INTERACTIVE_TOKEN);
    IPrincipal_put_RunLevel(pr, TASK_RUNLEVEL_HIGHEST);
    IPrincipal_Release(pr);
    SETB(ITaskDefinition_put_Data(d, t_), L"Some <data>");
    put("xml\r\n");
    ITaskDefinition_get_XmlText(d, &b);
    full_xml = SysAllocString(b);
    lines(b);
    ITaskDefinition_Release(d);
}
/* The full XML read into a new definition, then written again. */
static void roundtrip_step(void) {
    ITaskDefinition *d = NULL;
    ITriggerCollection *tc = NULL;
    IActionCollection *ac = NULL;
    ITrigger *t = NULL;
    BSTR b = NULL;
    LONG n = 0;
    TASK_TRIGGER_TYPE2 type = 0;
    int i, same = 1;
    ITaskService_NewTask(service, 0, &d);
    put("put");
    hr("hr", ITaskDefinition_put_XmlText(d, full_xml));
    ITaskDefinition_get_Triggers(d, &tc);
    ITriggerCollection_get_Count(tc, &n);
    field("triggers", n);
    put(" types=");
    for (i = 1; i <= n; i++) {
        if (ITriggerCollection_get_Item(tc, i, &t) == S_OK) {
            ITrigger_get_Type(t, &type);
            num(type);
            put(",");
            ITrigger_Release(t);
        }
    }
    ITriggerCollection_Release(tc);
    ITaskDefinition_get_Actions(d, &ac);
    IActionCollection_get_Count(ac, &n);
    field("actions", n);
    IActionCollection_Release(ac);
    ITaskDefinition_get_XmlText(d, &b);
    same = b && full_xml && wide_equal(b, full_xml);
    put(same ? " same=1" : " same=0");
    nl();
    if (!same) lines(b);
    else SysFreeString(b);
    /* Elements out of the schema's order, unknown elements. */
    put("schema");
    hr("unknown", ITaskDefinition_put_XmlText(d, b = s(L"<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><Bogus/></Task>")));
    SysFreeString(b);
    hr("minimal", ITaskDefinition_put_XmlText(d, b = s(L"<Task xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><Actions><Exec><Command>a.exe</Command></Exec></Actions></Task>")));
    SysFreeString(b);
    ITaskDefinition_get_XmlText(d, &b);
    nl();
    lines(b);
    ITaskDefinition_Release(d);
    SysFreeString(full_xml);
    full_xml = NULL;
}
static void ids(IDispatch *d, const char *label, const wchar_t *const *names, int count) {
    int i;
    put(label);
    for (i = 0; i < count; i++) {
        DISPID id = 0;
        LPOLESTR n = (LPOLESTR)names[i];
        HRESULT h = IDispatch_GetIDsOfNames(d, &iid_null, &n, 1, 0x409, &id);
        put(" ");
        wput(names[i]);
        put("=");
        if (h == S_OK)
            num(id);
        else
            hex((unsigned long)h);
    }
    nl();
}
static void dispatch_step(void) {
    static const wchar_t *const service_names[] = {L"GetFolder", L"GetRunningTasks", L"NewTask", L"Connect",
                                                   L"Connected", L"TargetServer", L"ConnectedUser",
                                                   L"ConnectedDomain", L"HighestVersion", L"Nothing"};
    static const wchar_t *const folder_names[] = {L"Name", L"Path", L"GetFolder", L"GetFolders", L"CreateFolder",
                                                  L"DeleteFolder", L"GetTask", L"GetTasks", L"DeleteTask",
                                                  L"RegisterTask", L"RegisterTaskDefinition"};
    static const wchar_t *const definition_names[] = {L"RegistrationInfo", L"Triggers", L"Settings", L"Data",
                                                      L"Principal", L"Actions", L"XmlText"};
    static const wchar_t *const trigger_names[] = {L"Type", L"Id", L"Repetition", L"ExecutionTimeLimit",
                                                   L"StartBoundary", L"EndBoundary", L"Enabled", L"RandomDelay"};
    static const wchar_t *const action_names[] = {L"Id", L"Type", L"Path", L"Arguments", L"WorkingDirectory"};
    IDispatch *d = NULL;
    ITypeInfo *ti = NULL;
    BSTR name = NULL;
    UINT count = 0;
    DISPPARAMS none = {NULL, NULL, 0, 0};
    VARIANT result;
    ITriggerCollection *tc = NULL;
    IActionCollection *ac = NULL;
    ITrigger *t = NULL;
    IAction *a = NULL;
    ITaskService_QueryInterface(service, &iid_dispatch, (void **)&d);
    put("typeinfo");
    hr("count", IDispatch_GetTypeInfoCount(d, &count));
    field("n", count);
    hr("get", IDispatch_GetTypeInfo(d, 0, 0x409, &ti));
    if (ti) {
        ITypeInfo_GetDocumentation(ti, MEMBERID_NIL, &name, NULL, NULL, NULL);
        text("name", name);
        ITypeInfo_Release(ti);
    }
    nl();
    ids(d, "service", service_names, 10);
    VariantInit(&result);
    put("invoke");
    hr("connected", IDispatch_Invoke(d, 5, &iid_null, 0x409, DISPATCH_PROPERTYGET, &none, &result, NULL, NULL));
    field("vt", V_VT(&result));
    field("value", V_BOOL(&result));
    VariantClear(&result);
    hr("highest", IDispatch_Invoke(d, 8, &iid_null, 0x409, DISPATCH_PROPERTYGET, &none, &result, NULL, NULL));
    field("vt", V_VT(&result));
    VariantClear(&result);
    nl();
    IDispatch_Release(d);
    ITaskFolder_QueryInterface(root, &iid_dispatch, (void **)&d);
    ids(d, "folder", folder_names, 11);
    IDispatch_Release(d);
    ITaskDefinition_QueryInterface(definition, &iid_dispatch, (void **)&d);
    ids(d, "definition", definition_names, 7);
    IDispatch_Release(d);
    ITaskDefinition_get_Triggers(definition, &tc);
    ITriggerCollection_get_Item(tc, 1, &t);
    ITrigger_QueryInterface(t, &iid_dispatch, (void **)&d);
    ids(d, "timetrigger", trigger_names, 8);
    IDispatch_Release(d);
    ITaskDefinition_get_Actions(definition, &ac);
    IActionCollection_get_Item(ac, 1, &a);
    IAction_QueryInterface(a, &iid_dispatch, (void **)&d);
    ids(d, "execaction", action_names, 5);
    IDispatch_Release(d);
    IAction_Release(a);
    ITrigger_Release(t);
    ITriggerCollection_Release(tc);
    IActionCollection_Release(ac);
}

/* ---- guest-only steps: registration changes the machine ------------------------------------ */
static int failed_check, check;
static HRESULT last;
#define EXPECT(c)                    \
    do {                             \
        check++;                     \
        if (!(c)) {                  \
            failed_check = check;    \
            return;                  \
        }                            \
    } while (0)
#define CALL(e, want)                    \
    do {                                 \
        last = (e);                      \
        EXPECT(last == (HRESULT)(want)); \
    } while (0)
static IRegisteredTask *task;
static long tasks_before;
static void register_step(void) {
    BSTR name = s(L"DoorsProbeTask"), b = NULL;
    VARIANT none = empty();
    IRegisteredTaskCollection *c = NULL;
    IRegisteredTask *again = NULL;
    TASK_STATE state = 0;
    VARIANT_BOOL enabled = 0;
    LONG result = 0, n = 0;
    DATE next = 0;
    SYSTEMTIME st;
    IActionCollection *ac = NULL;
    IAction *a = NULL;
    IExecAction *e = NULL;
    wchar_t self[MAX_PATH];
    GetModuleFileNameW(NULL, self, MAX_PATH);
    CALL(ITaskFolder_GetTasks(root, 0, &c), S_OK);
    CALL(IRegisteredTaskCollection_get_Count(c, &tasks_before), S_OK);
    IRegisteredTaskCollection_Release(c);
    /* The action runs this program, which exits with 7. */
    ITaskDefinition_get_Actions(definition, &ac);
    IActionCollection_get_Item(ac, 1, &a);
    IAction_QueryInterface(a, &iid_exec, (void **)&e);
    CALL(IExecAction_put_Path(e, b = s(self)), S_OK);
    SysFreeString(b);
    CALL(IExecAction_put_Arguments(e, b = s(L"exit7")), S_OK);
    SysFreeString(b);
    IExecAction_Release(e);
    IAction_Release(a);
    IActionCollection_Release(ac);
    CALL(ITaskFolder_RegisterTaskDefinition(root, name, definition, TASK_CREATE_OR_UPDATE, none, none,
                                            TASK_LOGON_INTERACTIVE_TOKEN, none, &task),
         S_OK);
    CALL(IRegisteredTask_get_Name(task, &b), S_OK);
    EXPECT(wide_equal(b, L"DoorsProbeTask"));
    SysFreeString(b);
    CALL(IRegisteredTask_get_Path(task, &b), S_OK);
    EXPECT(wide_equal(b, L"\\DoorsProbeTask"));
    SysFreeString(b);
    CALL(IRegisteredTask_get_State(task, &state), S_OK);
    EXPECT(state == TASK_STATE_READY);
    CALL(IRegisteredTask_get_Enabled(task, &enabled), S_OK);
    EXPECT(enabled == VARIANT_TRUE);
    CALL(IRegisteredTask_get_LastTaskResult(task, &result), S_OK);
    EXPECT(result == 0x41303);
    CALL(IRegisteredTask_get_NextRunTime(task, &next), S_OK);
    EXPECT(VariantTimeToSystemTime(next, &st) && st.wYear == 2030 && st.wMonth == 1 && st.wDay == 2 &&
           st.wHour == 3 && st.wMinute == 4);
    CALL(ITaskFolder_GetTask(root, name, &again), S_OK);
    IRegisteredTask_Release(again);
    CALL(ITaskFolder_GetTasks(root, 0, &c), S_OK);
    CALL(IRegisteredTaskCollection_get_Count(c, &n), S_OK);
    EXPECT(n == tasks_before + 1);
    IRegisteredTaskCollection_Release(c);
    /* Registering it again without UPDATE: already exists. */
    CALL(ITaskFolder_RegisterTaskDefinition(root, name, definition, TASK_CREATE, none, none,
                                            TASK_LOGON_INTERACTIVE_TOKEN, none, &again),
         HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS));
    CALL(IRegisteredTask_get_Xml(task, &b), S_OK);
    {
        const wchar_t *p = b;
        int found = 0;
        for (; *p; p++)
            if (p[0] == L'<' && p[1] == L'C' && p[2] == L'o' && p[3] == L'm') found = 1;
        EXPECT(found);
    }
    SysFreeString(b);
    SysFreeString(name);
}
static void run_step(void) {
    IRunningTask *r = NULL;
    LONG result = 0;
    DATE last_run = 0;
    TASK_STATE state = 0;
    VARIANT none = empty();
    int i;
    CALL(IRegisteredTask_Run(task, none, &r), S_OK);
    EXPECT(r != NULL);
    IRunningTask_Release(r);
    for (i = 0; i < 100; i++) {
        Sleep(100);
        IRegisteredTask_get_State(task, &state);
        IRegisteredTask_get_LastTaskResult(task, &result);
        if (state == TASK_STATE_READY && result != 0x41303 && result != 0x41301) break;
    }
    EXPECT(result == 7);
    CALL(IRegisteredTask_get_LastRunTime(task, &last_run), S_OK);
    EXPECT(last_run > 40000.0);
}
static void enable_step(void) {
    TASK_STATE state = 0;
    IRunningTask *r = NULL;
    VARIANT none = empty();
    CALL(IRegisteredTask_put_Enabled(task, VARIANT_FALSE), S_OK);
    CALL(IRegisteredTask_get_State(task, &state), S_OK);
    EXPECT(state == TASK_STATE_DISABLED);
    CALL(IRegisteredTask_Run(task, none, &r), SCHED_E_TASK_DISABLED);
    CALL(IRegisteredTask_put_Enabled(task, VARIANT_TRUE), S_OK);
    CALL(IRegisteredTask_get_State(task, &state), S_OK);
    EXPECT(state == TASK_STATE_READY);
}
static void delete_step(void) {
    BSTR name = s(L"DoorsProbeTask");
    IRegisteredTask *t = NULL;
    CALL(ITaskFolder_DeleteTask(root, name, 0), S_OK);
    CALL(ITaskFolder_GetTask(root, name, &t), HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND));
    IRegisteredTask_Release(task);
    task = NULL;
    SysFreeString(name);
}

typedef void (*Run)(void);
static const struct {
    const char *name;
    Run run;
    int guest_only;
} steps[] = {
    {"service", service_step, 0},     {"folder", folder_step, 0},   {"definition", definition_step, 0},
    {"triggers", triggers_step, 0},   {"actions", actions_step, 0}, {"xml", xml_step, 0},
    {"validate", validate_step, 0},   {"fullxml", fullxml_step, 0},   {"roundtrip", roundtrip_step, 0},
    {"dispatch", dispatch_step, 0}, {"register", register_step, 1},
    {"run", run_step, 1},             {"enable", enable_step, 1},   {"delete", delete_step, 1},
};
static const char *const expected[] = {
#include "tschedprobe.expected.h"
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
    return wide_equal(w, word);
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    int raw = last_word_is(line, L"raw"), guest = last_word_is(line, L"guest"), failed = 0, i;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    if (last_word_is(line, L"exit7")) ExitProcess(7);
    if (CoInitializeEx(NULL, COINIT_MULTITHREADED) != S_OK) {
        put("TSCHED-A FAILED CoInitializeEx\r\n");
        ExitProcess(1);
    }
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        if (steps[i].guest_only) {
            if (!guest || failed) continue;
            check = 0;
            failed_check = 0;
            steps[i].run();
            put("TSCHED-A ");
            num(i + 1);
            put(" ");
            put(steps[i].name);
            if (failed_check) {
                put(" FAILED check ");
                num(failed_check);
                put(" hr=");
                hex((unsigned long)last);
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
            put("TSCHED-A ");
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
    put(failed ? "TSCHED-A FAILED\r\n" : "TSCHED-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

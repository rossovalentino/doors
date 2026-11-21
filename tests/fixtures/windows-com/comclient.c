/* COMOOP-A (P16 M6): out-of-process COM from an ordinary Windows program
 * (Windows SDK, no C runtime). It registers comserver.exe as the
 * LocalServer32 of DoorsServer and doorsserver.tlb (per user), activates
 * the class (COM starts the server), calls the dual interface through the
 * automation marshaler (scalars, BSTR, VARIANT, SAFEARRAY, [in, out],
 * float and double, error info), through IDispatch, with a callback into
 * this apartment during a call (reentrancy), from another apartment
 * (RPC_E_WRONG_THREAD unmarshaled, working marshaled), across a server
 * crash (the call fails, the server's other objects are unreachable, a new
 * activation starts a new server), and unregisters. It writes the
 * registry, so it runs in the guest only. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define COBJMACROS
#include <windows.h>
#include <oleauto.h>
#include <intrin.h>

int _fltused = 0;
#pragma function(memcmp)
int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    size_t i;
    for (i = 0; i < n; i++)
        if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
    return 0;
}
#pragma function(memset)
void *memset(void *d, int v, size_t n) {
    __stosb((unsigned char *)d, (unsigned char)v, n);
    return d;
}

static const GUID clsid_server = {0xd0025b12, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
static const GUID iid_server = {0xd0025b11, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
static const GUID lib_server = {0xd0025b10, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
static const GUID iid_unknown = {0, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_dispatch = {0x00020400, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_null = {0, 0, 0, {0, 0, 0, 0, 0, 0, 0, 0}};
#define CLSID_KEY L"Software\\Classes\\CLSID\\{D0025B12-C0A1-4E5B-8F00-436F6D50726F}"

/* IDoorsServer's vtable (IDispatch's seven methods first). */
typedef struct IDoorsServer IDoorsServer;
typedef struct IDoorsServerVtbl {
    HRESULT(STDMETHODCALLTYPE *QueryInterface)(IDoorsServer *, REFIID, void **);
    ULONG(STDMETHODCALLTYPE *AddRef)(IDoorsServer *);
    ULONG(STDMETHODCALLTYPE *Release)(IDoorsServer *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfoCount)(IDoorsServer *, UINT *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfo)(IDoorsServer *, UINT, LCID, ITypeInfo **);
    HRESULT(STDMETHODCALLTYPE *GetIDsOfNames)(IDoorsServer *, REFIID, LPOLESTR *, UINT, LCID, DISPID *);
    HRESULT(STDMETHODCALLTYPE *Invoke)(IDoorsServer *, DISPID, REFIID, LCID, WORD, DISPPARAMS *, VARIANT *,
                                       EXCEPINFO *, UINT *);
    HRESULT(STDMETHODCALLTYPE *get_Pid)(IDoorsServer *, LONG *);
    HRESULT(STDMETHODCALLTYPE *get_Thread)(IDoorsServer *, LONG *);
    HRESULT(STDMETHODCALLTYPE *Add)(IDoorsServer *, LONG, LONG, LONG *);
    HRESULT(STDMETHODCALLTYPE *get_Name)(IDoorsServer *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *put_Name)(IDoorsServer *, BSTR);
    HRESULT(STDMETHODCALLTYPE *Echo)(IDoorsServer *, VARIANT, VARIANT *);
    HRESULT(STDMETHODCALLTYPE *Join)(IDoorsServer *, SAFEARRAY *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *Swap)(IDoorsServer *, BSTR *, LONG *);
    HRESULT(STDMETHODCALLTYPE *Fail)(IDoorsServer *);
    HRESULT(STDMETHODCALLTYPE *Callback)(IDoorsServer *, IDispatch *, LONG, LONG *);
    HRESULT(STDMETHODCALLTYPE *Crash)(IDoorsServer *);
    HRESULT(STDMETHODCALLTYPE *Scale)(IDoorsServer *, double, float, double *);
} IDoorsServerVtbl;
struct IDoorsServer {
    IDoorsServerVtbl *v;
};

/* ---- output -------------------------------------------------------------------------------- */
static HANDLE out;
static void put(const char *s) {
    DWORD w;
    int n = 0;
    while (s[n]) n++;
    WriteFile(out, s, (DWORD)n, &w, NULL);
}
static void number(long long v) {
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
static void hex(unsigned v) {
    char b[16];
    int i = 15;
    b[i] = 0;
    do {
        b[--i] = "0123456789abcdef"[v & 15];
        v >>= 4;
    } while (v);
    put("0x");
    put(b + i);
}
static int failed_step, check;
static HRESULT last;
#define EXPECT(cond)                       \
    do {                                   \
        check++;                           \
        if (!(cond)) return check;         \
    } while (0)
#define CALL(expr, want)                   \
    do {                                   \
        last = (expr);                     \
        check++;                           \
        if (last != (want)) return check;  \
    } while (0)
static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

/* ---- the callback sink (an IDispatch here) ------------------------------------------------- */
static DWORD sink_thread;
static LONG sink_calls;
static HRESULT STDMETHODCALLTYPE k_query(IDispatch *t, REFIID i, void **o) {
    if (IsEqualGUID(i, &iid_unknown) || IsEqualGUID(i, &iid_dispatch)) {
        *o = t;
        return S_OK;
    }
    *o = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE k_add(IDispatch *t) {
    (void)t;
    return 2;
}
static ULONG STDMETHODCALLTYPE k_release(IDispatch *t) {
    (void)t;
    return 1;
}
static HRESULT STDMETHODCALLTYPE k_count(IDispatch *t, UINT *n) {
    (void)t;
    *n = 0;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE k_info(IDispatch *t, UINT i, LCID l, ITypeInfo **o) {
    (void)t;
    (void)i;
    (void)l;
    *o = NULL;
    return E_NOTIMPL;
}
static HRESULT STDMETHODCALLTYPE k_ids(IDispatch *t, REFIID r, LPOLESTR *n, UINT c, LCID l, DISPID *ids) {
    (void)t;
    (void)r;
    (void)n;
    (void)c;
    (void)l;
    (void)ids;
    return DISP_E_UNKNOWNNAME;
}
/* DISPID 0: twice the argument. */
static HRESULT STDMETHODCALLTYPE k_invoke(IDispatch *t, DISPID id, REFIID r, LCID l, WORD f, DISPPARAMS *dp,
                                          VARIANT *res, EXCEPINFO *ei, UINT *arg) {
    (void)t;
    (void)r;
    (void)l;
    (void)f;
    (void)ei;
    (void)arg;
    if (id != 0 || !dp || dp->cArgs != 1 || V_VT(&dp->rgvarg[0]) != VT_I4) return DISP_E_MEMBERNOTFOUND;
    sink_thread = GetCurrentThreadId();
    sink_calls++;
    if (res) {
        VariantInit(res);
        V_VT(res) = VT_I4;
        V_I4(res) = V_I4(&dp->rgvarg[0]) * 2;
    }
    return S_OK;
}
static IDispatchVtbl sink_vtbl = {k_query, k_add, k_release, k_count, k_info, k_ids, k_invoke};
static struct {
    IDispatchVtbl *v;
} sink = {&sink_vtbl};

/* ---- steps --------------------------------------------------------------------------------- */
static wchar_t directory[MAX_PATH];
static IDoorsServer *server, *second;
static LONG server_pid;
static wchar_t *path_of(const wchar_t *name, wchar_t *buffer) {
    int n = 0, i;
    while (directory[n]) {
        buffer[n] = directory[n];
        n++;
    }
    for (i = 0; name[i]; i++) buffer[n++] = name[i];
    buffer[n] = 0;
    return buffer;
}
static int register_step(void) {
    HKEY key;
    wchar_t exe[MAX_PATH], tlb[MAX_PATH];
    ITypeLib *tl = NULL;
    path_of(L"comserver.exe", exe);
    path_of(L"doorsserver.tlb", tlb);
    EXPECT(RegCreateKeyExW(HKEY_CURRENT_USER, CLSID_KEY L"\\LocalServer32", 0, NULL, 0, KEY_ALL_ACCESS, NULL, &key,
                           NULL) == ERROR_SUCCESS);
    {
        DWORD n = 0;
        while (exe[n]) n++;
        EXPECT(RegSetValueExW(key, NULL, 0, REG_SZ, (const BYTE *)exe, (n + 1) * 2) == ERROR_SUCCESS);
    }
    RegCloseKey(key);
    CALL(LoadTypeLibEx(tlb, REGKIND_NONE, &tl), S_OK);
    CALL(RegisterTypeLibForUser(tl, tlb, NULL), S_OK);
    ITypeLib_Release(tl);
    return 0;
}
static int activate_step(void) {
    LONG thread = 0;
    IUnknown *a = NULL, *b = NULL;
    CALL(CoCreateInstance(&clsid_server, NULL, CLSCTX_LOCAL_SERVER, &iid_server, (void **)&server), S_OK);
    CALL(server->v->get_Pid(server, &server_pid), S_OK);
    EXPECT(server_pid != 0 && (DWORD)server_pid != GetCurrentProcessId());
    CALL(server->v->get_Thread(server, &thread), S_OK);
    /* Windows' thread IDs are unique system-wide; Doors' are per process
       (both main threads are 1), so only the server's own answer counts. */
    EXPECT(thread != 0);
    /* COM identity: one IUnknown, whatever interface asks. */
    CALL(server->v->QueryInterface(server, &iid_unknown, (void **)&a), S_OK);
    CALL(server->v->QueryInterface(server, &iid_dispatch, (void **)&b), S_OK);
    {
        IUnknown *c = NULL;
        CALL(IUnknown_QueryInterface(b, &iid_unknown, (void **)&c), S_OK);
        EXPECT(c == a);
        IUnknown_Release(c);
    }
    IUnknown_Release(b);
    IUnknown_Release(a);
    /* REGCLS_MULTIPLEUSE: a second object from the same server. */
    CALL(CoCreateInstance(&clsid_server, NULL, CLSCTX_LOCAL_SERVER, &iid_server, (void **)&second), S_OK);
    {
        LONG pid = 0;
        CALL(second->v->get_Pid(second, &pid), S_OK);
        EXPECT(pid == server_pid);
        EXPECT((void *)second != (void *)server);
    }
    return 0;
}
static int calls_step(void) {
    LONG sum = 0;
    BSTR name = NULL, joined = NULL;
    VARIANT in, copy;
    SAFEARRAY *sa;
    LONG i;
    static const wchar_t *words[] = {L"Macro", L"hard", L"Doors"};
    CALL(server->v->Add(server, 40, 2, &sum), S_OK);
    EXPECT(sum == 42);
    name = SysAllocString(L"Macrohard Doors");
    CALL(server->v->put_Name(server, name), S_OK);
    SysFreeString(name);
    name = NULL;
    CALL(server->v->get_Name(server, &name), S_OK);
    EXPECT(name && same(name, L"Macrohard Doors"));
    SysFreeString(name);
    VariantInit(&in);
    VariantInit(&copy);
    V_VT(&in) = VT_BSTR;
    V_BSTR(&in) = SysAllocString(L"echo");
    CALL(server->v->Echo(server, in, &copy), S_OK);
    EXPECT(V_VT(&copy) == VT_BSTR && same(V_BSTR(&copy), L"echo"));
    VariantClear(&in);
    VariantClear(&copy);
    V_VT(&in) = VT_DATE;
    V_DATE(&in) = 37623.5;
    CALL(server->v->Echo(server, in, &copy), S_OK);
    EXPECT(V_VT(&copy) == VT_DATE && V_DATE(&copy) == 37623.5);
    VariantClear(&copy);
    sa = SafeArrayCreateVector(VT_BSTR, 0, 3);
    for (i = 0; i < 3; i++) {
        BSTR w = SysAllocString(words[i]);
        SafeArrayPutElement(sa, &i, w);
        SysFreeString(w);
    }
    CALL(server->v->Join(server, sa, &joined), S_OK);
    EXPECT(joined && same(joined, L"Macro+hard+Doors"));
    SysFreeString(joined);
    /* An array inside a VARIANT comes back whole. */
    V_VT(&in) = VT_ARRAY | VT_BSTR;
    V_ARRAY(&in) = sa;
    CALL(server->v->Echo(server, in, &copy), S_OK);
    EXPECT(V_VT(&copy) == (VT_ARRAY | VT_BSTR));
    {
        BSTR third = NULL;
        i = 2;
        CALL(SafeArrayGetElement(V_ARRAY(&copy), &i, &third), S_OK);
        EXPECT(same(third, L"Doors"));
        SysFreeString(third);
    }
    VariantClear(&copy);
    SafeArrayDestroy(sa);
    return 0;
}
static int inout_step(void) {
    BSTR text = SysAllocString(L"sroD");
    LONG n = 21;
    double r = 0;
    CALL(server->v->Swap(server, &text, &n), S_OK);
    EXPECT(same(text, L"Dors") && n == 42);
    SysFreeString(text);
    CALL(server->v->Scale(server, 1.5, 4.0f, &r), S_OK);
    EXPECT(r == 6.0);
    return 0;
}
static int errorinfo_step(void) {
    IErrorInfo *e = NULL;
    BSTR text = NULL;
    CALL(server->v->Fail(server), E_FAIL);
    CALL(GetErrorInfo(0, &e), S_OK);
    EXPECT(e != NULL);
    IErrorInfo_GetDescription(e, &text);
    EXPECT(text && same(text, L"The server failed on purpose"));
    SysFreeString(text);
    IErrorInfo_GetSource(e, &text);
    EXPECT(text && same(text, L"Doors.Server"));
    SysFreeString(text);
    IErrorInfo_Release(e);
    return 0;
}
static int dispatch_step(void) {
    IDispatch *d = NULL;
    ITypeInfo *ti = NULL;
    BSTR tname = NULL;
    UINT count = 0;
    DISPID id = 0;
    LPOLESTR names[1] = {L"Add"};
    VARIANT args[2], res;
    DISPPARAMS dp;
    EXCEPINFO ei;
    CALL(server->v->QueryInterface(server, &iid_dispatch, (void **)&d), S_OK);
    CALL(IDispatch_GetTypeInfoCount(d, &count), S_OK);
    EXPECT(count == 1);
    CALL(IDispatch_GetTypeInfo(d, 0, 0x409, &ti), S_OK);
    ITypeInfo_GetDocumentation(ti, MEMBERID_NIL, &tname, NULL, NULL, NULL);
    EXPECT(tname && same(tname, L"IDoorsServer"));
    SysFreeString(tname);
    ITypeInfo_Release(ti);
    CALL(IDispatch_GetIDsOfNames(d, &iid_null, names, 1, 0x409, &id), S_OK);
    EXPECT(id == 3);
    V_VT(&args[1]) = VT_I4;
    V_I4(&args[1]) = 30;
    V_VT(&args[0]) = VT_BSTR;
    V_BSTR(&args[0]) = SysAllocString(L"12");
    dp.rgvarg = args;
    dp.cArgs = 2;
    dp.rgdispidNamedArgs = NULL;
    dp.cNamedArgs = 0;
    VariantInit(&res);
    memset(&ei, 0, sizeof ei);
    CALL(IDispatch_Invoke(d, id, &iid_null, 0x409, DISPATCH_METHOD, &dp, &res, &ei, NULL), S_OK);
    EXPECT(V_VT(&res) == VT_I4 && V_I4(&res) == 42);
    VariantClear(&args[0]);
    /* Fail takes no arguments: two are DISP_E_BADPARAMCOUNT, as Windows'
       ITypeInfo::Invoke answers. Then the failure through Invoke:
       DISP_E_EXCEPTION with the server's text. */
    CALL(IDispatch_Invoke(d, 8, &iid_null, 0x409, DISPATCH_METHOD, &dp, NULL, &ei, NULL), DISP_E_BADPARAMCOUNT);
    dp.cArgs = 0;
    CALL(IDispatch_Invoke(d, 8, &iid_null, 0x409, DISPATCH_METHOD, &dp, NULL, &ei, NULL), DISP_E_EXCEPTION);
    EXPECT(ei.scode == E_FAIL && ei.bstrDescription && same(ei.bstrDescription, L"The server failed on purpose"));
    SysFreeString(ei.bstrSource);
    SysFreeString(ei.bstrDescription);
    SysFreeString(ei.bstrHelpFile);
    IDispatch_Release(d);
    return 0;
}
static int callback_step(void) {
    LONG r = 0;
    sink_thread = 0;
    CALL(server->v->Callback(server, (IDispatch *)&sink, 21, &r), S_OK);
    /* The sink ran here, on this STA's thread, during the call. */
    EXPECT(r == 43 && sink_calls == 1 && sink_thread == GetCurrentThreadId());
    return 0;
}
static IStream *passed;
static HRESULT wrong, marshaled;
static LONG marshaled_sum;
static DWORD WINAPI worker(void *argument) {
    IDoorsServer *p = NULL;
    LONG sum = 0;
    (void)argument;
    CoInitialize(NULL);
    /* The main STA's proxy, used here: RPC_E_WRONG_THREAD. */
    wrong = server->v->Add(server, 1, 1, &sum);
    marshaled = CoGetInterfaceAndReleaseStream(passed, &iid_server, (void **)&p);
    if (marshaled == S_OK) {
        marshaled = p->v->Add(p, 2, 3, &marshaled_sum);
        p->v->Release(p);
    }
    CoUninitialize();
    return 0;
}
static int apartment_step(void) {
    HANDLE t;
    CALL(CoMarshalInterThreadInterfaceInStream(&iid_server, (IUnknown *)server, &passed), S_OK);
    t = CreateThread(NULL, 0, worker, NULL, 0, NULL);
    EXPECT(t != NULL);
    WaitForSingleObject(t, 30000);
    CloseHandle(t);
    last = wrong;
    EXPECT(wrong == RPC_E_WRONG_THREAD);
    last = marshaled;
    EXPECT(marshaled == S_OK && marshaled_sum == 5);
    return 0;
}
static int crash_step(void) {
    LONG sum = 0;
    last = second->v->Crash(second);
    check++;
    if (last != HRESULT_FROM_WIN32(RPC_S_CALL_FAILED) && last != HRESULT_FROM_WIN32(RPC_S_SERVER_UNAVAILABLE)) {
        return check;
    }
    /* The server is gone: its other object is unreachable. */
    last = server->v->Add(server, 1, 1, &sum);
    EXPECT(last == HRESULT_FROM_WIN32(RPC_S_SERVER_UNAVAILABLE) || last == HRESULT_FROM_WIN32(RPC_S_CALL_FAILED));
    second->v->Release(second);
    server->v->Release(server);
    second = server = NULL;
    return 0;
}
static int relaunch_step(void) {
    LONG pid = 0, sum = 0;
    IDoorsServer *p = NULL;
    CALL(CoCreateInstance(&clsid_server, NULL, CLSCTX_LOCAL_SERVER, &iid_server, (void **)&p), S_OK);
    CALL(p->v->get_Pid(p, &pid), S_OK);
    EXPECT(pid != 0 && pid != server_pid);
    CALL(p->v->Add(p, 2, 2, &sum), S_OK);
    EXPECT(sum == 4);
    p->v->Release(p);
    return 0;
}
static int unregister_step(void) {
    CALL(UnRegisterTypeLibForUser(&lib_server, 1, 0, 0, SYS_WIN64), S_OK);
    EXPECT(RegDeleteTreeW(HKEY_CURRENT_USER, CLSID_KEY) == ERROR_SUCCESS);
    {
        IDoorsServer *p = NULL;
        CALL(CoCreateInstance(&clsid_server, NULL, CLSCTX_LOCAL_SERVER, &iid_server, (void **)&p),
             REGDB_E_CLASSNOTREG);
    }
    return 0;
}

static const struct {
    const char *name;
    int (*run)(void);
} steps[] = {
    {"register", register_step}, {"activate", activate_step}, {"calls", calls_step},
    {"inout", inout_step},       {"errorinfo", errorinfo_step}, {"dispatch", dispatch_step},
    {"callback", callback_step}, {"apartment", apartment_step}, {"crash", crash_step},
    {"relaunch", relaunch_step}, {"unregister", unregister_step},
};

void entry(void) {
    DWORD n, i;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    n = GetModuleFileNameW(NULL, directory, MAX_PATH);
    while (n > 0 && directory[n - 1] != L'\\' && directory[n - 1] != L'/') n--;
    directory[n] = 0;
    CoInitialize(NULL);
    for (i = 0; i < sizeof steps / sizeof steps[0]; i++) {
        int failed;
        check = 0;
        last = 0;
        failed = steps[i].run();
        put("COMOOP-A ");
        number(i + 1);
        put(" ");
        put(steps[i].name);
        if (failed) {
            put(" FAILED check ");
            number(failed);
            put(" hr=");
            hex((unsigned)last);
            put("\r\n");
            if (!failed_step) failed_step = (int)i + 1;
            break;
        }
        put(" ok ");
        number(i == 1 ? server_pid : check);
        put("\r\n");
    }
    CoUninitialize();
    put(failed_step ? "COMOOP-A FAILED\r\n" : "COMOOP-A PASSED\r\n");
    ExitProcess(failed_step ? 1 : 0);
}

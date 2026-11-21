/* COMOOP-A's local server (P16 M6): an ordinary Windows program (Windows
 * SDK, no C runtime) that COM starts with -Embedding. Its single-threaded
 * apartment registers the DoorsServer class object (REGCLS_MULTIPLEUSE)
 * and runs a message loop, as an STA server does, until its last object
 * goes. IDoorsServer is the dual interface doorsserver.tlb describes (the
 * type library beside the program); IDispatch goes through DispInvoke on
 * its vtable-side type information. Crash ends the process in the middle
 * of the call. */
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
static const GUID iid_classfactory = {0x00000001, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_supporterrorinfo = {0xDF0B3D60, 0x548F, 0x101B,
                                          {0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19}};
static const GUID iid_errorinfo = {0x1CF2B120, 0x547D, 0x101B, {0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19}};
static const GUID iid_null = {0, 0, 0, {0, 0, 0, 0, 0, 0, 0, 0}};

static ITypeInfo *info; /* IDoorsServer's vtable side */
static LONG objects, locks;
static DWORD main_thread;
static void maybe_quit(void) {
    if (objects == 0 && locks == 0) PostThreadMessageW(main_thread, WM_QUIT, 0, 0);
}

typedef struct Server Server;
typedef struct ServerVtbl {
    HRESULT(STDMETHODCALLTYPE *QueryInterface)(Server *, REFIID, void **);
    ULONG(STDMETHODCALLTYPE *AddRef)(Server *);
    ULONG(STDMETHODCALLTYPE *Release)(Server *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfoCount)(Server *, UINT *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfo)(Server *, UINT, LCID, ITypeInfo **);
    HRESULT(STDMETHODCALLTYPE *GetIDsOfNames)(Server *, REFIID, LPOLESTR *, UINT, LCID, DISPID *);
    HRESULT(STDMETHODCALLTYPE *Invoke)(Server *, DISPID, REFIID, LCID, WORD, DISPPARAMS *, VARIANT *, EXCEPINFO *, UINT *);
    HRESULT(STDMETHODCALLTYPE *get_Pid)(Server *, LONG *);
    HRESULT(STDMETHODCALLTYPE *get_Thread)(Server *, LONG *);
    HRESULT(STDMETHODCALLTYPE *Add)(Server *, LONG, LONG, LONG *);
    HRESULT(STDMETHODCALLTYPE *get_Name)(Server *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *put_Name)(Server *, BSTR);
    HRESULT(STDMETHODCALLTYPE *Echo)(Server *, VARIANT, VARIANT *);
    HRESULT(STDMETHODCALLTYPE *Join)(Server *, SAFEARRAY *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *Swap)(Server *, BSTR *, LONG *);
    HRESULT(STDMETHODCALLTYPE *Fail)(Server *);
    HRESULT(STDMETHODCALLTYPE *Callback)(Server *, IDispatch *, LONG, LONG *);
    HRESULT(STDMETHODCALLTYPE *Crash)(Server *);
    HRESULT(STDMETHODCALLTYPE *Scale)(Server *, double, float, double *);
} ServerVtbl;
typedef struct SupportVtbl {
    HRESULT(STDMETHODCALLTYPE *QueryInterface)(void *, REFIID, void **);
    ULONG(STDMETHODCALLTYPE *AddRef)(void *);
    ULONG(STDMETHODCALLTYPE *Release)(void *);
    HRESULT(STDMETHODCALLTYPE *InterfaceSupportsErrorInfo)(void *, REFIID);
} SupportVtbl;
struct Server {
    ServerVtbl *vtbl;
    SupportVtbl *support;
    LONG refs;
    BSTR name;
};

static HRESULT STDMETHODCALLTYPE s_query(Server *t, REFIID i, void **o) {
    if (IsEqualGUID(i, &iid_supporterrorinfo)) {
        *o = &t->support;
    } else if (IsEqualGUID(i, &iid_unknown) || IsEqualGUID(i, &iid_dispatch) || IsEqualGUID(i, &iid_server)) {
        *o = t;
    } else {
        *o = NULL;
        return E_NOINTERFACE;
    }
    InterlockedIncrement(&t->refs);
    return S_OK;
}
static ULONG STDMETHODCALLTYPE s_add(Server *t) { return (ULONG)InterlockedIncrement(&t->refs); }
static ULONG STDMETHODCALLTYPE s_release(Server *t) {
    LONG left = InterlockedDecrement(&t->refs);
    if (left == 0) {
        SysFreeString(t->name);
        HeapFree(GetProcessHeap(), 0, t);
        InterlockedDecrement(&objects);
        maybe_quit();
    }
    return (ULONG)left;
}
static HRESULT STDMETHODCALLTYPE s_count(Server *t, UINT *n) {
    (void)t;
    *n = 1;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_typeinfo(Server *t, UINT i, LCID l, ITypeInfo **o) {
    (void)t;
    (void)l;
    if (i != 0) return DISP_E_BADINDEX;
    ITypeInfo_AddRef(info);
    *o = info;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_ids(Server *t, REFIID r, LPOLESTR *n, UINT c, LCID l, DISPID *ids) {
    (void)t;
    (void)r;
    (void)l;
    return DispGetIDsOfNames(info, n, c, ids);
}
static HRESULT STDMETHODCALLTYPE s_invoke(Server *t, DISPID id, REFIID r, LCID l, WORD f, DISPPARAMS *dp, VARIANT *res,
                                          EXCEPINFO *ei, UINT *arg) {
    (void)r;
    (void)l;
    return DispInvoke(t, info, id, f, dp, res, ei, arg);
}
static HRESULT STDMETHODCALLTYPE s_pid(Server *t, LONG *v) {
    (void)t;
    *v = (LONG)GetCurrentProcessId();
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_thread(Server *t, LONG *v) {
    (void)t;
    *v = (LONG)GetCurrentThreadId();
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_addn(Server *t, LONG a, LONG b, LONG *s) {
    (void)t;
    *s = a + b;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_get_name(Server *t, BSTR *v) {
    *v = SysAllocString(t->name);
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_put_name(Server *t, BSTR v) {
    SysFreeString(t->name);
    t->name = SysAllocString(v);
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_echo(Server *t, VARIANT v, VARIANT *c) {
    (void)t;
    VariantInit(c);
    return VariantCopy(c, &v);
}
static HRESULT STDMETHODCALLTYPE s_join(Server *t, SAFEARRAY *parts, BSTR *j) {
    LONG lb, ub, i;
    wchar_t buf[128];
    int n = 0;
    (void)t;
    SafeArrayGetLBound(parts, 1, &lb);
    SafeArrayGetUBound(parts, 1, &ub);
    for (i = lb; i <= ub; i++) {
        BSTR s = NULL;
        UINT k;
        SafeArrayGetElement(parts, &i, &s);
        for (k = 0; k < SysStringLen(s) && n < 120; k++) buf[n++] = s[k];
        if (i < ub) buf[n++] = L'+';
        SysFreeString(s);
    }
    *j = SysAllocStringLen(buf, (UINT)n);
    return S_OK;
}
/* [in, out]: the text reversed, the number doubled. */
static HRESULT STDMETHODCALLTYPE s_swap(Server *t, BSTR *text, LONG *number) {
    UINT n = SysStringLen(*text), k;
    BSTR r = SysAllocStringLen(NULL, n);
    (void)t;
    for (k = 0; k < n; k++) r[k] = (*text)[n - 1 - k];
    SysFreeString(*text);
    *text = r;
    *number *= 2;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_fail(Server *t) {
    ICreateErrorInfo *c = NULL;
    IErrorInfo *e = NULL;
    (void)t;
    CreateErrorInfo(&c);
    ICreateErrorInfo_SetDescription(c, L"The server failed on purpose");
    ICreateErrorInfo_SetSource(c, L"Doors.Server");
    ICreateErrorInfo_SetGUID(c, &iid_server);
    ICreateErrorInfo_QueryInterface(c, &iid_errorinfo, (void **)&e);
    SetErrorInfo(0, e);
    IErrorInfo_Release(e);
    ICreateErrorInfo_Release(c);
    return E_FAIL;
}
/* Calls back into the client (its DISPID 0) in the middle of this call. */
static HRESULT STDMETHODCALLTYPE s_callback(Server *t, IDispatch *sink, LONG value, LONG *result) {
    DISPPARAMS dp;
    VARIANT arg, res;
    HRESULT h;
    (void)t;
    VariantInit(&arg);
    VariantInit(&res);
    V_VT(&arg) = VT_I4;
    V_I4(&arg) = value;
    dp.rgvarg = &arg;
    dp.cArgs = 1;
    dp.rgdispidNamedArgs = NULL;
    dp.cNamedArgs = 0;
    h = IDispatch_Invoke(sink, 0, &iid_null, 0x409, DISPATCH_METHOD, &dp, &res, NULL, NULL);
    if (h != S_OK) return h;
    if (V_VT(&res) != VT_I4) return E_UNEXPECTED;
    *result = V_I4(&res) + 1;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE s_crash(Server *t) {
    (void)t;
    ExitProcess(3);
}
static HRESULT STDMETHODCALLTYPE s_scale(Server *t, double x, float f, double *r) {
    (void)t;
    *r = x * f;
    return S_OK;
}
static ServerVtbl server_vtbl = {s_query,  s_add,      s_release,  s_count,   s_typeinfo, s_ids,      s_invoke,
                                 s_pid,    s_thread,   s_addn,     s_get_name, s_put_name, s_echo,    s_join,
                                 s_swap,   s_fail,     s_callback, s_crash,   s_scale};
static HRESULT STDMETHODCALLTYPE e_query(void *t, REFIID i, void **o) {
    return s_query((Server *)((char *)t - sizeof(void *)), i, o);
}
static ULONG STDMETHODCALLTYPE e_add(void *t) { return s_add((Server *)((char *)t - sizeof(void *))); }
static ULONG STDMETHODCALLTYPE e_release(void *t) { return s_release((Server *)((char *)t - sizeof(void *))); }
static HRESULT STDMETHODCALLTYPE e_supports(void *t, REFIID i) {
    (void)t;
    return IsEqualGUID(i, &iid_server) ? S_OK : S_FALSE;
}
static SupportVtbl support_vtbl = {e_query, e_add, e_release, e_supports};

/* The class object. */
typedef struct Factory {
    IClassFactoryVtbl *vtbl;
} Factory;
static HRESULT STDMETHODCALLTYPE f_query(IClassFactory *t, REFIID i, void **o) {
    if (IsEqualGUID(i, &iid_unknown) || IsEqualGUID(i, &iid_classfactory)) {
        *o = t;
        return S_OK;
    }
    *o = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE f_add(IClassFactory *t) {
    (void)t;
    return 2;
}
static ULONG STDMETHODCALLTYPE f_release(IClassFactory *t) {
    (void)t;
    return 1;
}
static HRESULT STDMETHODCALLTYPE f_create(IClassFactory *t, IUnknown *outer, REFIID i, void **o) {
    Server *s;
    HRESULT h;
    (void)t;
    *o = NULL;
    if (outer) return CLASS_E_NOAGGREGATION;
    s = (Server *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof *s);
    if (!s) return E_OUTOFMEMORY;
    s->vtbl = &server_vtbl;
    s->support = &support_vtbl;
    s->refs = 1;
    s->name = SysAllocString(L"server");
    InterlockedIncrement(&objects);
    h = s_query(s, i, o);
    s_release(s);
    return h;
}
static HRESULT STDMETHODCALLTYPE f_lock(IClassFactory *t, BOOL lock) {
    (void)t;
    if (lock) {
        InterlockedIncrement(&locks);
    } else {
        InterlockedDecrement(&locks);
        maybe_quit();
    }
    return S_OK;
}
static IClassFactoryVtbl factory_vtbl = {f_query, f_add, f_release, f_create, f_lock};
static Factory factory = {&factory_vtbl};

static void say(const char *s) {
    DWORD w;
    int n = 0;
    while (s[n]) n++;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, (DWORD)n, &w, NULL);
}

void entry(void) {
    const wchar_t *line = GetCommandLineW();
    wchar_t path[MAX_PATH];
    DWORD n, cookie = 0, i;
    int embedding = 0;
    ITypeLib *tl = NULL;
    ITypeInfo *dual = NULL;
    HREFTYPE href;
    MSG msg;
    for (i = 0; line[i]; i++) {
        if ((line[i] == L'-' || line[i] == L'/') && line[i + 1] == L'E' && line[i + 2] == L'm') embedding = 1;
    }
    if (!embedding) {
        say("comserver.exe: COM starts this program (-Embedding)\r\n");
        ExitProcess(2);
    }
    main_thread = GetCurrentThreadId();
    if (FAILED(CoInitialize(NULL))) ExitProcess(4);
    /* doorsserver.tlb beside the program. */
    n = GetModuleFileNameW(NULL, path, MAX_PATH);
    while (n > 0 && path[n - 1] != L'\\' && path[n - 1] != L'/') n--;
    {
        const wchar_t *name = L"doorsserver.tlb";
        for (i = 0; name[i] && n < MAX_PATH - 1; i++) path[n++] = name[i];
        path[n] = 0;
    }
    /* The registered type library, else the one beside the program. Each
     * failure has its own exit code. */
    if (LoadRegTypeLib(&lib_server, 1, 0, 0, &tl) != S_OK && LoadTypeLibEx(path, REGKIND_NONE, &tl) != S_OK) {
        ExitProcess(5);
    }
    if (ITypeLib_GetTypeInfoOfGuid(tl, &iid_server, &dual) != S_OK) ExitProcess(7);
    if (ITypeInfo_GetRefTypeOfImplType(dual, (UINT)-1, &href) != S_OK) ExitProcess(8);
    if (ITypeInfo_GetRefTypeInfo(dual, href, &info) != S_OK) ExitProcess(9);
    if (CoRegisterClassObject(&clsid_server, (IUnknown *)&factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE,
                              &cookie) != S_OK) {
        ExitProcess(6);
    }
    /* A server nobody uses (no objects, no locks) leaves after a minute. */
    SetTimer(NULL, 0, 60000, NULL);
    for (;;) {
        BOOL got = GetMessageW(&msg, NULL, 0, 0);
        if (got < 0) ExitProcess(3000 + GetLastError());
        if (got == 0) break;
        if (msg.message == WM_TIMER && msg.hwnd == NULL && objects == 0 && locks == 0) ExitProcess(11);
        DispatchMessageW(&msg);
    }
    CoRevokeClassObject(cookie);
    ITypeInfo_Release(info);
    ITypeInfo_Release(dual);
    ITypeLib_Release(tl);
    CoUninitialize();
    ExitProcess(0);
}

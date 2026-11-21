/* COM-A probe (P16): an ordinary Windows console program (Windows SDK, no C
 * runtime) that uses in-process COM through ole32.dll:
 *
 *   comprobe.exe
 *
 * Apartments and their rules, GUID helpers, the task allocator, a class
 * object the program registers itself, DirectSound activated through
 * CoCreateInstance (its InprocServer32 registration) and its Initialize
 * rule, ProgIDs, an IStream over HGLOBAL, PROPVARIANT copies and the
 * implicit MTA seen from a second thread. It makes no sound and changes no
 * setting, so it runs on Windows as reference.
 *
 * One "COM-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define COBJMACROS
#include <windows.h>
#include <objbase.h>
#include <propidl.h>
#include <mmsystem.h>
#include <dsound.h>
#include <intrin.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
    return destination;
}

static const GUID iid_unknown = {0x00000000, 0x0000, 0x0000, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_class_factory = {0x00000001, 0x0000, 0x0000, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_stream = {0x0000000C, 0x0000, 0x0000, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID clsid_directsound8 = {0x3901cc3f, 0x84b5, 0x4fa4, {0xba, 0x35, 0xaa, 0x81, 0x72, 0xb8, 0xa0, 0x9b}};
static const GUID iid_directsound8 = {0xc50a7e93, 0xf395, 0x4834, {0x9e, 0xf6, 0x7f, 0xa9, 0x9d, 0xe5, 0x09, 0x66}};
/* The probe's own class: {D00250D1-C0A1-4E5B-8F00-436F6D50726F}. */
static const GUID default_playback = {0xdef00000, 0x9c6d, 0x47ed, {0xaa, 0xf1, 0x4d, 0xda, 0x8f, 0x2b, 0x5c, 0x03}};
static const GUID clsid_probe = {0xd00250d1, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};

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
    unsigned long u = v < 0 ? (unsigned long)(-v) : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u && i > 1);
    if (v < 0) b[--i] = '-';
    put(b + i);
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("COM-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static int same(const void *a, const void *b, int n) {
    const unsigned char *x = (const unsigned char *)a, *y = (const unsigned char *)b;
    for (int i = 0; i < n; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}
static int wide_same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

/* ---- the probe's class: an object counting its references ---- */
static volatile LONG object_refs, objects_made;
static HRESULT STDMETHODCALLTYPE o_query(IUnknown *self, REFIID iid, void **out_object) {
    if (same(iid, &iid_unknown, 16)) {
        *out_object = self;
        InterlockedIncrement(&object_refs);
        return S_OK;
    }
    *out_object = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE o_add_ref(IUnknown *self) {
    (void)self;
    return (ULONG)InterlockedIncrement(&object_refs);
}
static ULONG STDMETHODCALLTYPE o_release(IUnknown *self) {
    (void)self;
    return (ULONG)InterlockedDecrement(&object_refs);
}
static IUnknownVtbl object_vtbl = {o_query, o_add_ref, o_release};
static IUnknown object = {&object_vtbl};

static volatile LONG factory_refs;
static HRESULT STDMETHODCALLTYPE f_query(IClassFactory *self, REFIID iid, void **out_object) {
    if (same(iid, &iid_unknown, 16) || same(iid, &iid_class_factory, 16)) {
        *out_object = self;
        InterlockedIncrement(&factory_refs);
        return S_OK;
    }
    *out_object = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE f_add_ref(IClassFactory *self) {
    (void)self;
    return (ULONG)InterlockedIncrement(&factory_refs);
}
static ULONG STDMETHODCALLTYPE f_release(IClassFactory *self) {
    (void)self;
    return (ULONG)InterlockedDecrement(&factory_refs);
}
static HRESULT STDMETHODCALLTYPE f_create(IClassFactory *self, IUnknown *outer, REFIID iid, void **out_object) {
    (void)self;
    if (outer) return CLASS_E_NOAGGREGATION;
    InterlockedIncrement(&objects_made);
    return o_query(&object, iid, out_object);
}
static HRESULT STDMETHODCALLTYPE f_lock(IClassFactory *self, BOOL lock) {
    (void)self;
    (void)lock;
    return S_OK;
}
static IClassFactoryVtbl factory_vtbl = {f_query, f_add_ref, f_release, f_create, f_lock};
static IClassFactory factory = {&factory_vtbl};

/* Stream data (static: no stack probe without a C runtime). */
static unsigned char data[5000], read_back[5000];
static volatile LONG worker_type = -9, worker_qualifier = -9;
static volatile HRESULT worker_result;
static volatile HRESULT worker_before, worker_create;
static DWORD WINAPI worker(LPVOID unused) {
    (void)unused;
    APTTYPE type;
    APTTYPEQUALIFIER qualifier;
    /* Never initialized: the thread is in the implicit MTA. */
    worker_before = CoGetApartmentType(&type, &qualifier);
    /* Activation still works in the implicit MTA. */
    IDirectSound8 *ds = NULL;
    worker_create = CoCreateInstance(&clsid_directsound8, NULL, CLSCTX_INPROC_SERVER, &iid_directsound8, (void **)&ds);
    if (ds) IDirectSound8_Release(ds);
    worker_result = CoGetApartmentType(&type, &qualifier);
    if (worker_result == S_OK) {
        worker_type = type;
        worker_qualifier = qualifier;
    }
    return 0;
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);

    /* 1: an STA, counted; the other mode refused; the main STA. */
    HRESULT a = CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    HRESULT b = CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    HRESULT c = CoInitializeEx(NULL, COINIT_MULTITHREADED);
    APTTYPE type = APTTYPE_CURRENT;
    APTTYPEQUALIFIER qualifier = APTTYPEQUALIFIER_NONE;
    HRESULT t = CoGetApartmentType(&type, &qualifier);
    step(1, "apartment", a == S_OK && b == S_FALSE && c == RPC_E_CHANGED_MODE && t == S_OK && type == APTTYPE_MAINSTA, (long)type);
    /* Each successful call is balanced (the refused one is not counted). */
    CoUninitialize();
    CoUninitialize();

    /* 2: GUIDs: random version-4 values, text forms both ways. */
    GUID g1, g2, back;
    HRESULT n1 = CoCreateGuid(&g1), n2 = CoCreateGuid(&g2);
    wchar_t text[40];
    int chars = StringFromGUID2(&g1, text, 40);
    HRESULT p = CLSIDFromString(text, &back);
    IID iid;
    HRESULT bad = IIDFromString(L"{00000000-0000-0000-0000-00000000000G}", &iid);
    LPOLESTR owned = NULL;
    HRESULT s = StringFromCLSID(&clsid_probe, &owned);
    int formatted = s == S_OK && owned && wide_same(owned, L"{D00250D1-C0A1-4E5B-8F00-436F6D50726F}");
    CoTaskMemFree(owned);
    step(2, "guids",
         n1 == S_OK && n2 == S_OK && !same(&g1, &g2, 16) && (g1.Data3 >> 12) == 4 && chars == 39 && p == S_OK && same(&g1, &back, 16) &&
             bad == CO_E_IIDSTRING && formatted,
         (long)chars);

    /* 3: the task allocator and IMalloc. */
    void *block = CoTaskMemAlloc(100);
    block = CoTaskMemRealloc(block, 1000);
    IMalloc *malloc_ = NULL;
    HRESULT m = CoGetMalloc(1, &malloc_);
    SIZE_T size = malloc_ ? IMalloc_GetSize(malloc_, block) : 0;
    int did = malloc_ ? IMalloc_DidAlloc(malloc_, block) : 0;
    CoTaskMemFree(block);
    if (malloc_) IMalloc_Release(malloc_);
    step(3, "task-memory", block && m == S_OK && size >= 1000 && did == 1, (long)size);

    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    /* 4: a registered class object serves CoCreateInstance until revoked. */
    DWORD cookie = 0;
    HRESULT r = CoRegisterClassObject(&clsid_probe, (IUnknown *)&factory, CLSCTX_INPROC_SERVER, REGCLS_MULTIPLEUSE, &cookie);
    IUnknown *made = NULL;
    HRESULT ci = CoCreateInstance(&clsid_probe, NULL, CLSCTX_INPROC_SERVER, &iid_unknown, (void **)&made);
    LONG with_object = object_refs;
    if (made) IUnknown_Release(made);
    HRESULT rv = CoRevokeClassObject(cookie);
    IUnknown *gone = NULL;
    HRESULT after = CoCreateInstance(&clsid_probe, NULL, CLSCTX_INPROC_SERVER, &iid_unknown, (void **)&gone);
    step(4, "class-object",
         r == S_OK && ci == S_OK && made == &object && with_object == 1 && object_refs == 0 && objects_made == 1 && rv == S_OK &&
             after == REGDB_E_CLASSNOTREG && factory_refs == 0,
         (long)after);

    /* 5: DirectSound through its registration: uninitialized until
     *    Initialize, which works once. */
    IDirectSound8 *ds = NULL;
    HRESULT d = CoCreateInstance(&clsid_directsound8, NULL, CLSCTX_INPROC_SERVER, &iid_directsound8, (void **)&ds);
    DSCAPS caps;
    memset(&caps, 0, sizeof caps);
    caps.dwSize = sizeof caps;
    HRESULT early = ds ? IDirectSound8_GetCaps(ds, &caps) : E_FAIL;
    HRESULT init = ds ? IDirectSound8_Initialize(ds, NULL) : E_FAIL;
    HRESULT late = ds ? IDirectSound8_GetCaps(ds, &caps) : E_FAIL;
    HRESULT again = ds ? IDirectSound8_Initialize(ds, NULL) : E_FAIL;
    ULONG left = ds ? IDirectSound8_Release(ds) : 99;
    /* This program does not import dsound.dll: COM loaded it. Its
     * GetDeviceID, found by name in the loaded module, maps the default
     * device to a real GUID. */
    HMODULE dsound = GetModuleHandleW(L"dsound.dll");
    typedef HRESULT(WINAPI * GetDeviceIdFn)(LPCGUID, LPGUID);
    GetDeviceIdFn get_device_id = dsound ? (GetDeviceIdFn)(void *)GetProcAddress(dsound, "GetDeviceID") : NULL;
    GUID device;
    HRESULT id = get_device_id ? get_device_id(&default_playback, &device) : E_FAIL;
    /* LoadLibrary by full path of another system DLL nothing imports. */
    HMODULE winmm = LoadLibraryW(L"C:\\Windows\\System32\\winmm.dll");
    DWORD load_error = winmm ? 0 : GetLastError();
    typedef UINT(WINAPI * NumDevsFn)(void);
    NumDevsFn num_devs = winmm ? (NumDevsFn)(void *)GetProcAddress(winmm, "waveOutGetNumDevs") : NULL;
    UINT outputs = num_devs ? num_devs() : 0;
    BOOL freed = winmm ? FreeLibrary(winmm) : FALSE;
    /* The detail names what failed: one bit per check. */
    long sound_failures = (d != S_OK) | (early != DSERR_UNINITIALIZED) << 1 | (init != DS_OK) << 2 | (late != DS_OK) << 3 |
                          (again != DSERR_ALREADYINITIALIZED) << 4 | (left != 0) << 5 | (id != DS_OK) << 6 |
                          (outputs < 1) << 7 | !freed << 8 | !winmm << 9 | !dsound << 10 |
                          (long)(load_error & 0xffff) << 12;
    step(5, "directsound", sound_failures == 0, sound_failures);

    /* 6: ProgIDs: an unknown one, and a class registered without one. */
    CLSID none;
    HRESULT pid = CLSIDFromProgID(L"Doors.NoSuchClass.1", &none);
    LPOLESTR name = NULL;
    HRESULT reverse = ProgIDFromCLSID(&clsid_directsound8, &name);
    step(6, "progid", pid == CO_E_CLASSSTRING && reverse == REGDB_E_CLASSNOTREG && name == NULL, (long)pid);

    /* 7: an IStream over HGLOBAL: write, seek, read back, stat, clone,
     *    shrink, and its HGLOBAL. */
    IStream *stream = NULL;
    HRESULT cs = CreateStreamOnHGlobal(NULL, TRUE, &stream);
    for (int i = 0; i < 5000; i++) data[i] = (unsigned char)(i * 7);
    ULONG moved = 0, got = 0;
    LARGE_INTEGER zero;
    zero.QuadPart = 0;
    ULARGE_INTEGER at;
    at.QuadPart = 1;
    STATSTG stat;
    memset(&stat, 0, sizeof stat);
    HGLOBAL global = NULL;
    IStream *copy = NULL;
    HRESULT w = stream ? IStream_Write(stream, data, 5000, &moved) : E_FAIL;
    HRESULT sk = stream ? IStream_Seek(stream, zero, STREAM_SEEK_SET, &at) : E_FAIL;
    HRESULT rd = stream ? IStream_Read(stream, read_back, 5000, &got) : E_FAIL;
    HRESULT st = stream ? IStream_Stat(stream, &stat, STATFLAG_NONAME) : E_FAIL;
    HRESULT cl = stream ? IStream_Clone(stream, &copy) : E_FAIL;
    ULONG copied = 0;
    unsigned char tail[10];
    if (copy) {
        LARGE_INTEGER back10;
        back10.QuadPart = -10;
        IStream_Seek(copy, back10, STREAM_SEEK_END, NULL);
        IStream_Read(copy, tail, 10, &copied);
        IStream_Release(copy);
    }
    ULARGE_INTEGER ten;
    ten.QuadPart = 10;
    STATSTG shrunk;
    memset(&shrunk, 0, sizeof shrunk);
    HRESULT ss = stream ? IStream_SetSize(stream, ten) : E_FAIL;
    if (stream) IStream_Stat(stream, &shrunk, STATFLAG_NONAME);
    HRESULT gh = stream ? GetHGlobalFromStream(stream, &global) : E_FAIL;
    IUnknown *as_unknown = NULL;
    HRESULT qi = stream ? IStream_QueryInterface(stream, &iid_stream, (void **)&as_unknown) : E_FAIL;
    if (as_unknown) IUnknown_Release(as_unknown);
    ULONG stream_left = stream ? IStream_Release(stream) : 99;
    step(7, "hglobal-stream",
         cs == S_OK && w == S_OK && moved == 5000 && sk == S_OK && at.QuadPart == 0 && rd == S_OK && got == 5000 && same(data, read_back, 5000) &&
             st == S_OK && stat.cbSize.QuadPart == 5000 && stat.type == STGTY_STREAM && cl == S_OK && copied == 10 && same(tail, data + 4990, 10) &&
             ss == S_OK && shrunk.cbSize.QuadPart == 10 && gh == S_OK && global != NULL && qi == S_OK && stream_left == 0,
         (long)stat.cbSize.QuadPart);

    /* 8: PROPVARIANT: a string copied deeply, an interface referenced,
     *    both cleared. */
    PROPVARIANT pv, pv2, unk;
    PropVariantInit(&pv);
    PropVariantInit(&pv2);
    PropVariantInit(&unk);
    pv.vt = VT_LPWSTR;
    pv.pwszVal = (LPWSTR)CoTaskMemAlloc(12);
    memset(pv.pwszVal, 0, 12);
    pv.pwszVal[0] = L'D';
    pv.pwszVal[1] = L'o';
    pv.pwszVal[2] = L'o';
    pv.pwszVal[3] = L'r';
    pv.pwszVal[4] = L's';
    HRESULT pc = PropVariantCopy(&pv2, &pv);
    int deep = pc == S_OK && pv2.vt == VT_LPWSTR && pv2.pwszVal != pv.pwszVal && wide_same(pv2.pwszVal, L"Doors");
    unk.vt = VT_UNKNOWN;
    unk.punkVal = &object;
    IUnknown_AddRef(&object);
    HRESULT c1 = PropVariantClear(&pv), c2 = PropVariantClear(&pv2), c3 = PropVariantClear(&unk);
    step(8, "propvariant", deep && c1 == S_OK && c2 == S_OK && c3 == S_OK && pv.vt == VT_EMPTY && unk.vt == VT_EMPTY && object_refs == 0,
         (long)object_refs);

    /* 9: after the last CoUninitialize nothing activates; the thread may
     *    then join the MTA, and a thread that never initialized COM is in
     *    it implicitly. */
    CoUninitialize();
    IUnknown *late_object = NULL;
    HRESULT none_left = CoCreateInstance(&clsid_directsound8, NULL, CLSCTX_INPROC_SERVER, &iid_directsound8, (void **)&late_object);
    HRESULT mta = CoInitializeEx(NULL, COINIT_MULTITHREADED);
    HRESULT tt = CoGetApartmentType(&type, &qualifier);
    HANDLE thread = CreateThread(NULL, 0, worker, NULL, 0, NULL);
    DWORD waited = thread ? WaitForSingleObject(thread, 5000) : WAIT_FAILED;
    if (thread) CloseHandle(thread);
    CoUninitialize();
    step(9, "implicit-mta",
         none_left == CO_E_NOTINITIALIZED && mta == S_OK && tt == S_OK && type == APTTYPE_MTA && waited == WAIT_OBJECT_0 && worker_type == APTTYPE_MTA &&
             worker_qualifier == APTTYPEQUALIFIER_IMPLICIT_MTA && worker_before == S_OK && worker_create == S_OK,
         (long)worker_type * 10 + worker_qualifier);

    put(failed_step ? "COM-A FAILED\r\n" : "COM-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

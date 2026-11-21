/* OLEAUT-A probe (P16 M6): an ordinary Windows console program (Windows
 * SDK, no C runtime) that prints what oleaut32.dll answers, line by line,
 * for comparison with Windows' own answers (oleautprobe.reference.txt):
 *
 *   oleautprobe.exe <doorsprobe.tlb>
 *
 * BSTR layout, VARIANT clear/copy/change-type, SAFEARRAY descriptors and
 * elements, dates, error info, the type library MIDL built from
 * doorsprobe.idl (every TYPEATTR, FUNCDESC, VARDESC and name), and
 * IDispatch: ITypeInfo::Invoke on a dual object (coercion, defaults,
 * optional arguments, named arguments, property put, exceptions), its
 * IDs of names, CreateStdDispatch and DispGetParam. LCID 0x409; nothing
 * printed depends on addresses. It loads the type library without
 * registering it and changes nothing, so it runs on Windows as reference. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define COBJMACROS
#include <windows.h>
#include <oleauto.h>
#include <shellapi.h>
#include <winreg.h>
#include <intrin.h>

int _fltused = 0;
#pragma function(memset)
void *memset(void *d, int v, size_t n) {
    __stosb((unsigned char *)d, (unsigned char)v, n);
    return d;
}
#pragma function(memcmp)
int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    size_t i;
    for (i = 0; i < n; i++)
        if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
    return 0;
}
#pragma function(memcpy)
void *memcpy(void *d, const void *s, size_t n) {
    __movsb((unsigned char *)d, (const unsigned char *)s, n);
    return d;
}

/* uuid.lib's IIDs, here (the toolchain pins no uuid.lib). */
static const GUID iid_null = {0, 0, 0, {0, 0, 0, 0, 0, 0, 0, 0}};
static const GUID iid_unknown = {0, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_dispatch = {0x00020400, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static const GUID iid_errorinfo = {0x1CF2B120, 0x547D, 0x101B, {0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19}};
static const GUID iid_supporterrorinfo = {0xDF0B3D60, 0x548F, 0x101B,
                                          {0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19}};

static HANDLE out;
/* A step's output is captured and compared with Windows' (put appends). */
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
    int i = 31;
    int neg = v < 0;
    unsigned long long u = neg ? (unsigned long long)-v : (unsigned long long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (neg) b[--i] = '-';
    put(b + i);
}
static void hex(unsigned long long v) {
    char b[24];
    int i = 23;
    b[i] = 0;
    do {
        b[--i] = "0123456789abcdef"[v & 15];
        v >>= 4;
    } while (v);
    put("0x");
    put(b + i);
}
static void bytes(const void *p, int n) {
    const unsigned char *b = p;
    int i;
    for (i = 0; i < n; i++) {
        char t[3];
        t[0] = "0123456789abcdef"[b[i] >> 4];
        t[1] = "0123456789abcdef"[b[i] & 15];
        t[2] = 0;
        put(t);
    }
}
static void wput(const wchar_t *s, int n) {
    char b[600];
    int k;
    if (!s) {
        put("(null)");
        return;
    }
    k = WideCharToMultiByte(CP_UTF8, 0, s, n, b, sizeof b - 1, NULL, NULL);
    if (k > 0) {
        b[k] = 0;
        put(b);
    }
}
static void bput(BSTR b) {
    if (!b) {
        put("(null)");
        return;
    }
    put("\"");
    wput(b, (int)SysStringLen(b));
    put("\"");
}
static void nl(void) { put("\r\n"); }
static void guid(const GUID *g) { bytes(g, 16); }
static void show(VARIANT *v) {
    put("vt=");
    num(V_VT(v));
    put(" ");
    switch (V_VT(v)) {
    case VT_BSTR: bput(V_BSTR(v)); break;
    case VT_I2: num(V_I2(v)); break;
    case VT_I4: case VT_INT: num(V_I4(v)); break;
    case VT_UI1: num(V_UI1(v)); break;
    case VT_BOOL: num(V_BOOL(v)); break;
    case VT_ERROR: hex((unsigned)V_ERROR(v)); break;
    case VT_R8: case VT_DATE: case VT_CY: bytes(&v->llVal, 8); break;
    case VT_R4: bytes(&v->fltVal, 4); break;
    case VT_EMPTY: case VT_NULL: break;
    default: put("?"); break;
    }
}

/* ---- a counting object for VARIANT reference rules ---------------------------- */
typedef struct Counted {
    IUnknownVtbl *vtbl;
    LONG refs;
} Counted;
static HRESULT STDMETHODCALLTYPE c_query(IUnknown *t, REFIID i, void **o) {
    (void)i;
    *o = t;
    t->lpVtbl->AddRef(t);
    return S_OK;
}
static ULONG STDMETHODCALLTYPE c_add(IUnknown *t) { return (ULONG)++((Counted *)t)->refs; }
static ULONG STDMETHODCALLTYPE c_release(IUnknown *t) { return (ULONG)--((Counted *)t)->refs; }
static IUnknownVtbl counted_vtbl = {c_query, c_add, c_release};

static void bstrs(void) {
    BSTR a = SysAllocString(L"Doors");
    BSTR b = SysAllocStringByteLen("abc", 3);
    BSTR c = SysAllocStringLen(NULL, 4);
    BSTR d = SysAllocStringLen(L"abcdef", 3);
    put("bstr a len=");
    num(SysStringLen(a));
    put(" bytes=");
    num(SysStringByteLen(a));
    put(" prefix=");
    bytes((char *)a - 4, 4);
    put(" tail=");
    bytes((char *)a + 10, 2);
    nl();
    put("bstr b len=");
    num(SysStringLen(b));
    put(" bytes=");
    num(SysStringByteLen(b));
    put(" data=");
    bytes(b, 5);
    nl();
    put("bstr c len=");
    num(SysStringLen(c));
    put(" d=");
    bput(d);
    put(" null=");
    num(SysAllocString(NULL) == NULL);
    put(" nulllen=");
    num(SysStringLen(NULL));
    nl();
    put("bstr realloc ");
    num(SysReAllocString(&a, L"Macrohard Doors"));
    put(" ");
    bput(a);
    put(" ");
    num(SysReAllocStringLen(&a, a, 9));
    put(" ");
    bput(a);
    nl();
    SysFreeString(a);
    SysFreeString(b);
    SysFreeString(c);
    SysFreeString(d);
    SysFreeString(NULL);
}

static void variants(void) {
    VARIANT a, b, ref;
    Counted obj = {&counted_vtbl, 1};
    LONG target = 7;
    HRESULT h;
    VariantInit(&a);
    VariantInit(&b);
    V_VT(&a) = VT_BSTR;
    V_BSTR(&a) = SysAllocString(L"copy me");
    h = VariantCopy(&b, &a);
    put("variant copy ");
    hex((unsigned)h);
    put(" ");
    show(&b);
    put(" distinct=");
    num(V_BSTR(&a) != V_BSTR(&b));
    nl();
    VariantClear(&a);
    VariantClear(&b);
    V_VT(&a) = VT_UNKNOWN;
    V_UNKNOWN(&a) = (IUnknown *)&obj;
    VariantCopy(&b, &a);
    put("variant unknown refs-after-copy=");
    num(obj.refs);
    VariantClear(&b);
    put(" after-clear=");
    num(obj.refs);
    put(" vt=");
    num(V_VT(&b));
    nl();
    V_VT(&ref) = VT_I4 | VT_BYREF;
    V_I4REF(&ref) = &target;
    VariantInit(&b);
    h = VariantCopyInd(&b, &ref);
    put("variant copyind ");
    hex((unsigned)h);
    put(" ");
    show(&b);
    h = VariantChangeType(&b, &ref, 0, VT_BSTR);
    put(" byref-to-bstr ");
    hex((unsigned)h);
    put(" ");
    show(&b);
    nl();
    VariantClear(&b);
    V_VT(&a) = 0x7fff;
    h = VariantClear(&a);
    put("variant clear-bad ");
    hex((unsigned)h);
    put(" vt=");
    num(V_VT(&a));
    nl();
    VariantInit(&a);
    V_VT(&a) = VT_R8;
    V_R8(&a) = 2.5;
    h = VariantChangeType(&a, &a, 0, VT_I4);
    put("variant in-place ");
    hex((unsigned)h);
    put(" ");
    show(&a);
    h = VariantChangeType(&a, &a, 0, VT_ARRAY | VT_I4);
    put(" to-array ");
    hex((unsigned)h);
    h = VariantChangeType(&a, &a, 0, VT_VARIANT);
    put(" to-variant ");
    hex((unsigned)h);
    nl();
}

static void arrays(void) {
    SAFEARRAYBOUND bounds[2] = {{3, 1}, {2, -1}};
    SAFEARRAY *sa = SafeArrayCreate(VT_BSTR, 2, bounds);
    LONG idx[2], lb, ub, val;
    BSTR in = SysAllocString(L"cell"), got = NULL;
    VARTYPE vt;
    HRESULT h;
    void *data, *cell;
    put("array dims=");
    num(SafeArrayGetDim(sa));
    put(" elem=");
    num(SafeArrayGetElemsize(sa));
    put(" features=");
    hex(sa->fFeatures);
    put(" stored=");
    num(sa->rgsabound[0].cElements);
    put("/");
    num(sa->rgsabound[0].lLbound);
    put(",");
    num(sa->rgsabound[1].cElements);
    put("/");
    num(sa->rgsabound[1].lLbound);
    SafeArrayGetLBound(sa, 1, &lb);
    SafeArrayGetUBound(sa, 1, &ub);
    put(" dim1=");
    num(lb);
    put("..");
    num(ub);
    SafeArrayGetLBound(sa, 2, &lb);
    SafeArrayGetUBound(sa, 2, &ub);
    put(" dim2=");
    num(lb);
    put("..");
    num(ub);
    put(" dim3=");
    hex((unsigned)SafeArrayGetLBound(sa, 3, &lb));
    SafeArrayGetVartype(sa, &vt);
    put(" vt=");
    num(vt);
    nl();
    idx[0] = 3;
    idx[1] = 0;
    h = SafeArrayPutElement(sa, idx, in);
    put("array put ");
    hex((unsigned)h);
    h = SafeArrayGetElement(sa, idx, &got);
    put(" get ");
    hex((unsigned)h);
    put(" copy=");
    num(got != in);
    put(" ");
    bput(got);
    SafeArrayPtrOfIndex(sa, idx, &cell);
    SafeArrayAccessData(sa, &data);
    put(" offset=");
    num(((char *)cell - (char *)data) / 8);
    put(" locks=");
    num(sa->cLocks);
    SafeArrayUnaccessData(sa);
    idx[1] = 1;
    put(" bad ");
    hex((unsigned)SafeArrayPutElement(sa, idx, in));
    nl();
    SafeArrayLock(sa);
    put("array destroy-locked ");
    hex((unsigned)SafeArrayDestroy(sa));
    SafeArrayUnlock(sa);
    put(" unlock-again ");
    hex((unsigned)SafeArrayUnlock(sa));
    put(" destroy ");
    hex((unsigned)SafeArrayDestroy(sa));
    nl();
    {
        SAFEARRAY *v = SafeArrayCreateVector(VT_I4, 0, 4), *copy = NULL;
        SAFEARRAYBOUND nb = {6, 0};
        LONG i;
        put("vector features=");
        hex(v->fFeatures);
        put(" elem=");
        num(v->cbElements);
        put(" inline=");
        num((char *)v->pvData == (char *)v + sizeof(SAFEARRAY));
        for (i = 0; i < 4; i++) {
            val = i * 10;
            SafeArrayPutElement(v, &i, &val);
        }
        put(" redim ");
        hex((unsigned)SafeArrayRedim(v, &nb));
        put(" n=");
        num(v->rgsabound[0].cElements);
        SafeArrayCopy(v, &copy);
        i = 3;
        SafeArrayGetElement(copy, &i, &val);
        put(" copy3=");
        num(val);
        i = 5;
        SafeArrayGetElement(copy, &i, &val);
        put(" copy5=");
        num(val);
        put(" copyfeatures=");
        hex(copy->fFeatures);
        nl();
        SafeArrayDestroy(v);
        SafeArrayDestroy(copy);
    }
    {
        SAFEARRAY *u = SafeArrayCreateVector(VT_VARIANT, 1, 2), *d = NULL, *k = NULL;
        SAFEARRAYBOUND one = {2, 0};
        GUID iid;
        put("variant-vector features=");
        hex(u->fFeatures);
        put(" elem=");
        num(u->cbElements);
        SafeArrayAllocDescriptorEx(VT_UNKNOWN, 1, &d);
        put(" desc-unknown features=");
        hex(d->fFeatures);
        put(" elem=");
        num(d->cbElements);
        SafeArrayGetIID(d, &iid);
        put(" iid=");
        guid(&iid);
        k = SafeArrayCreate(VT_DISPATCH, 1, &one);
        put(" dispatch features=");
        hex(k->fFeatures);
        nl();
        SafeArrayDestroy(u);
        SafeArrayDestroyDescriptor(d);
        SafeArrayDestroy(k);
    }
    SysFreeString(in);
    SysFreeString(got);
}

static void dates(void) {
    SYSTEMTIME st;
    double d;
    WORD dd, dt;
    UDATE ud;
    memset(&st, 0, sizeof st);
    st.wYear = 2003;
    st.wMonth = 1;
    st.wDay = 2;
    st.wHour = 4;
    st.wMinute = 5;
    st.wSecond = 6;
    st.wMilliseconds = 700;
    put("date st2v ");
    num(SystemTimeToVariantTime(&st, &d));
    put(" ");
    bytes(&d, 8);
    memset(&st, 0, sizeof st);
    put(" v2st ");
    num(VariantTimeToSystemTime(37623.6713541667, &st));
    put(" ");
    num(st.wYear);
    put("-");
    num(st.wMonth);
    put("-");
    num(st.wDay);
    put(" ");
    num(st.wHour);
    put(":");
    num(st.wMinute);
    put(":");
    num(st.wSecond);
    put(" dow=");
    num(st.wDayOfWeek);
    nl();
    put("date dos ");
    num(VariantTimeToDosDateTime(37623.170208333335, &dd, &dt));
    put(" ");
    hex(dd);
    put(" ");
    hex(dt);
    put(" back ");
    num(DosDateTimeToVariantTime(dd, dt, &d));
    put(" ");
    bytes(&d, 8);
    put(" udate ");
    hex((unsigned)VarUdateFromDate(37623.170208333335, 0, &ud));
    put(" doy=");
    num(ud.wDayOfYear);
    nl();
    st.wYear = 2003;
    st.wMonth = 2;
    st.wDay = 30;
    put("date bad ");
    num(SystemTimeToVariantTime(&st, &d));
    nl();
}

static void errors(void) {
    ICreateErrorInfo *create = NULL;
    IErrorInfo *info = NULL, *got = NULL;
    BSTR text = NULL;
    GUID g;
    DWORD ctx = 0;
    HRESULT h = CreateErrorInfo(&create);
    put("error create ");
    hex((unsigned)h);
    ICreateErrorInfo_SetDescription(create, L"Something failed");
    ICreateErrorInfo_SetSource(create, L"Doors.Probe");
    ICreateErrorInfo_SetHelpContext(create, 42);
    ICreateErrorInfo_QueryInterface(create, &iid_errorinfo, (void **)&info);
    put(" set ");
    hex((unsigned)SetErrorInfo(0, info));
    put(" get ");
    hex((unsigned)GetErrorInfo(0, &got));
    put(" same=");
    num(got == info);
    IErrorInfo_GetDescription(got, &text);
    put(" ");
    bput(text);
    SysFreeString(text);
    IErrorInfo_GetSource(got, &text);
    put(" ");
    bput(text);
    SysFreeString(text);
    IErrorInfo_GetHelpContext(got, &ctx);
    put(" ctx=");
    num(ctx);
    IErrorInfo_GetGUID(got, &g);
    put(" guid=");
    guid(&g);
    put(" again ");
    hex((unsigned)GetErrorInfo(0, &info));
    nl();
    IErrorInfo_Release(got);
    ICreateErrorInfo_Release(create);
}

/* ---- the type library ------------------------------------------------------- */
static void tdesc(ITypeInfo *ti, TYPEDESC *t) {
    num(t->vt);
    if (t->vt == VT_PTR || t->vt == VT_SAFEARRAY) {
        put("(");
        tdesc(ti, t->lptdesc);
        put(")");
    } else if (t->vt == VT_USERDEFINED) {
        ITypeInfo *r = NULL;
        BSTR name = NULL;
        put("(");
        if (ITypeInfo_GetRefTypeInfo(ti, t->hreftype, &r) == S_OK) {
            ITypeInfo_GetDocumentation(r, MEMBERID_NIL, &name, NULL, NULL, NULL);
            wput(name, -1);
            SysFreeString(name);
            ITypeInfo_Release(r);
        } else {
            put("?");
        }
        put(")");
    } else if (t->vt == VT_CARRAY) {
        put("[]");
    }
}
static void docs(ITypeInfo *ti, MEMBERID id) {
    BSTR name = NULL, doc = NULL, file = NULL;
    DWORD ctx = 0;
    HRESULT h = ITypeInfo_GetDocumentation(ti, id, &name, &doc, &ctx, &file);
    put(" doc ");
    hex((unsigned)h);
    put(" ");
    bput(name);
    put(" ");
    bput(doc);
    put(" ctx=");
    num(ctx);
    put(" file=");
    bput(file);
    SysFreeString(name);
    SysFreeString(doc);
    SysFreeString(file);
}
static void names(ITypeInfo *ti, MEMBERID id) {
    BSTR list[16];
    UINT n = 0, i;
    HRESULT h = ITypeInfo_GetNames(ti, id, list, 16, &n);
    put(" names ");
    hex((unsigned)h);
    for (i = 0; i < n; i++) {
        put(" ");
        wput(list[i], -1);
        SysFreeString(list[i]);
    }
}
static void typeinfo(ITypeInfo *ti, const char *label) {
    TYPEATTR *ta = NULL;
    UINT i;
    ITypeInfo_GetTypeAttr(ti, &ta);
    put(label);
    put(" guid=");
    guid(&ta->guid);
    put(" kind=");
    num(ta->typekind);
    put(" funcs=");
    num(ta->cFuncs);
    put(" vars=");
    num(ta->cVars);
    put(" impl=");
    num(ta->cImplTypes);
    put(" vft=");
    num(ta->cbSizeVft);
    put(" size=");
    num(ta->cbSizeInstance);
    put(" align=");
    num(ta->cbAlignment);
    put(" flags=");
    hex(ta->wTypeFlags);
    put(" ver=");
    num(ta->wMajorVerNum);
    put(".");
    num(ta->wMinorVerNum);
    put(" lcid=");
    num(ta->lcid);
    put(" ctor=");
    num(ta->memidConstructor);
    if (ta->typekind == TKIND_ALIAS) {
        put(" alias=");
        tdesc(ti, &ta->tdescAlias);
    }
    docs(ti, MEMBERID_NIL);
    nl();
    for (i = 0; i < ta->cImplTypes; i++) {
        HREFTYPE href = 0;
        INT flags = 0;
        ITypeInfo *r = NULL;
        BSTR name = NULL;
        ITypeInfo_GetRefTypeOfImplType(ti, i, &href);
        ITypeInfo_GetImplTypeFlags(ti, i, &flags);
        ITypeInfo_GetRefTypeInfo(ti, href, &r);
        ITypeInfo_GetDocumentation(r, MEMBERID_NIL, &name, NULL, NULL, NULL);
        put(label);
        put(" impl ");
        num(i);
        put(" ");
        wput(name, -1);
        put(" flags=");
        num(flags);
        nl();
        SysFreeString(name);
        ITypeInfo_Release(r);
    }
    for (i = 0; i < ta->cFuncs; i++) {
        FUNCDESC *fd = NULL;
        SHORT p;
        ITypeInfo_GetFuncDesc(ti, i, &fd);
        put(label);
        put(" func ");
        num(i);
        put(" id=");
        num(fd->memid);
        put(" kind=");
        num(fd->funckind);
        put(" inv=");
        num(fd->invkind);
        put(" cc=");
        num(fd->callconv);
        put(" params=");
        num(fd->cParams);
        put(" opt=");
        num(fd->cParamsOpt);
        put(" vft=");
        num(fd->oVft);
        put(" flags=");
        hex(fd->wFuncFlags);
        put(" ret=");
        tdesc(ti, &fd->elemdescFunc.tdesc);
        names(ti, fd->memid);
        for (p = 0; p < fd->cParams; p++) {
            PARAMDESC *pd = &fd->lprgelemdescParam[p].paramdesc;
            put(" [");
            tdesc(ti, &fd->lprgelemdescParam[p].tdesc);
            put(" f=");
            hex(pd->wParamFlags);
            if (pd->wParamFlags & PARAMFLAG_FHASDEFAULT) {
                put(" def=");
                show(&pd->pparamdescex->varDefaultValue);
            }
            put("]");
        }
        docs(ti, fd->memid);
        if (fd->funckind == FUNC_STATIC) {
            BSTR dll = NULL, entry_name = NULL;
            WORD ordinal = 0;
            HRESULT eh = ITypeInfo_GetDllEntry(ti, fd->memid, fd->invkind, &dll, &entry_name, &ordinal);
            put(" entry ");
            hex((unsigned)eh);
            put(" ");
            bput(dll);
            put(" ");
            bput(entry_name);
            put(" ");
            num(ordinal);
            SysFreeString(dll);
            SysFreeString(entry_name);
        }
        nl();
        ITypeInfo_ReleaseFuncDesc(ti, fd);
    }
    for (i = 0; i < ta->cVars; i++) {
        VARDESC *vd = NULL;
        ITypeInfo_GetVarDesc(ti, i, &vd);
        put(label);
        put(" var ");
        num(i);
        put(" id=");
        num(vd->memid);
        put(" kind=");
        num(vd->varkind);
        put(" flags=");
        hex(vd->wVarFlags);
        put(" type=");
        tdesc(ti, &vd->elemdescVar.tdesc);
        if (vd->varkind == VAR_CONST) {
            put(" value=");
            show(vd->lpvarValue);
        } else {
            put(" offset=");
            num(vd->oInst);
        }
        names(ti, vd->memid);
        nl();
        ITypeInfo_ReleaseVarDesc(ti, vd);
    }
    ITypeInfo_ReleaseTypeAttr(ti, ta);
}
static ITypeInfo *probe_info;   /* IDoorsProbe, the vtable (TKIND_INTERFACE) side */
static ITypeInfo *probe_dispinfo; /* IDoorsProbe as TKIND_DISPATCH */
static void typelib(const wchar_t *path) {
    ITypeLib *tl = NULL;
    TLIBATTR *la = NULL;
    BSTR name = NULL, doc = NULL;
    UINT i, n;
    HRESULT h = LoadTypeLibEx(path, REGKIND_NONE, &tl);
    put("tlb load ");
    hex((unsigned)h);
    nl();
    if (h != S_OK) return;
    ITypeLib_GetLibAttr(tl, &la);
    put("tlb lib guid=");
    guid(&la->guid);
    put(" lcid=");
    num(la->lcid);
    put(" sys=");
    num(la->syskind);
    put(" ver=");
    num(la->wMajorVerNum);
    put(".");
    num(la->wMinorVerNum);
    put(" flags=");
    hex(la->wLibFlags);
    ITypeLib_ReleaseTLibAttr(tl, la);
    ITypeLib_GetDocumentation(tl, -1, &name, &doc, NULL, NULL);
    put(" ");
    bput(name);
    put(" ");
    bput(doc);
    SysFreeString(name);
    SysFreeString(doc);
    n = ITypeLib_GetTypeInfoCount(tl);
    put(" count=");
    num(n);
    nl();
    for (i = 0; i < n; i++) {
        ITypeInfo *ti = NULL;
        TYPEKIND kind;
        char label[16] = "tlb t?";
        label[5] = (char)('0' + i);
        ITypeLib_GetTypeInfoType(tl, i, &kind);
        ITypeLib_GetTypeInfo(tl, i, &ti);
        typeinfo(ti, label);
        if (i == 2) {
            HREFTYPE href;
            ITypeInfo_AddRef(ti);
            probe_dispinfo = ti;
            /* The dual interface's vtable side. */
            if (ITypeInfo_GetRefTypeOfImplType(ti, (UINT)-1, &href) == S_OK &&
                ITypeInfo_GetRefTypeInfo(ti, href, &probe_info) == S_OK) {
                typeinfo(probe_info, "tlb t2v");
            }
        }
        ITypeInfo_Release(ti);
    }
    {
        static const GUID counter = {0xd0025a14, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
        static const GUID missing = {0xd0025aff, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
        ITypeInfo *ti = NULL;
        BOOL found = FALSE;
        wchar_t probe_name[] = L"idoorsprobe";
        put("tlb ofguid ");
        hex((unsigned)ITypeLib_GetTypeInfoOfGuid(tl, &counter, &ti));
        if (ti) ITypeInfo_Release(ti);
        ti = NULL;
        put(" missing ");
        hex((unsigned)ITypeLib_GetTypeInfoOfGuid(tl, &missing, &ti));
        put(" isname ");
        hex((unsigned)ITypeLib_IsName(tl, probe_name, 0, &found));
        put(" ");
        num(found);
        put(" ");
        wput(probe_name, -1);
        nl();
    }
    /* stdole2, as the dual interface references it. */
    if (probe_info) {
        HREFTYPE href;
        ITypeInfo *disp = NULL, *unk = NULL;
        ITypeLib *std = NULL;
        TLIBATTR *sa = NULL;
        UINT k = 99, count;
        ITypeInfo_GetRefTypeOfImplType(probe_info, 0, &href);
        ITypeInfo_GetRefTypeInfo(probe_info, href, &disp);
        typeinfo(disp, "std d");
        ITypeInfo_GetRefTypeOfImplType(disp, 0, &href);
        ITypeInfo_GetRefTypeInfo(disp, href, &unk);
        typeinfo(unk, "std u");
        ITypeInfo_GetContainingTypeLib(disp, &std, &k);
        ITypeLib_GetLibAttr(std, &sa);
        put("std lib guid=");
        guid(&sa->guid);
        put(" lcid=");
        num(sa->lcid);
        put(" sys=");
        num(sa->syskind);
        put(" ver=");
        num(sa->wMajorVerNum);
        put(".");
        num(sa->wMinorVerNum);
        put(" flags=");
        hex(sa->wLibFlags);
        ITypeLib_ReleaseTLibAttr(std, sa);
        count = ITypeLib_GetTypeInfoCount(std);
        put(" count=");
        num(count);
        put(" index=");
        num(k);
        nl();
        for (k = 0; k < count; k++) {
            BSTR tname = NULL;
            TYPEKIND kind;
            ITypeLib_GetTypeInfoType(std, k, &kind);
            ITypeLib_GetDocumentation(std, (INT)k, &tname, NULL, NULL, NULL);
            put("std t");
            num(k);
            put(" kind=");
            num(kind);
            put(" ");
            wput(tname, -1);
            nl();
            SysFreeString(tname);
        }
        ITypeLib_Release(std);
        ITypeInfo_Release(unk);
        ITypeInfo_Release(disp);
    }
    ITypeLib_Release(tl);
}

/* ---- a dual object ------------------------------------------------------------ */
typedef struct Probe Probe;
struct ProbeVtbl {
    HRESULT(STDMETHODCALLTYPE *QueryInterface)(Probe *, REFIID, void **);
    ULONG(STDMETHODCALLTYPE *AddRef)(Probe *);
    ULONG(STDMETHODCALLTYPE *Release)(Probe *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfoCount)(Probe *, UINT *);
    HRESULT(STDMETHODCALLTYPE *GetTypeInfo)(Probe *, UINT, LCID, ITypeInfo **);
    HRESULT(STDMETHODCALLTYPE *GetIDsOfNames)(Probe *, REFIID, LPOLESTR *, UINT, LCID, DISPID *);
    HRESULT(STDMETHODCALLTYPE *Invoke)(Probe *, DISPID, REFIID, LCID, WORD, DISPPARAMS *, VARIANT *, EXCEPINFO *, UINT *);
    HRESULT(STDMETHODCALLTYPE *get_Name)(Probe *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *put_Name)(Probe *, BSTR);
    HRESULT(STDMETHODCALLTYPE *Add)(Probe *, LONG, LONG, LONG *);
    HRESULT(STDMETHODCALLTYPE *Echo)(Probe *, VARIANT, VARIANT *);
    HRESULT(STDMETHODCALLTYPE *Scale)(Probe *, double, double, double *);
    HRESULT(STDMETHODCALLTYPE *Fail)(Probe *);
    HRESULT(STDMETHODCALLTYPE *Join)(Probe *, SAFEARRAY *, BSTR *);
    HRESULT(STDMETHODCALLTYPE *get_Color)(Probe *, LONG *);
    HRESULT(STDMETHODCALLTYPE *Count)(Probe *, VARIANT, LONG *);
};
typedef struct SupportVtbl {
    HRESULT(STDMETHODCALLTYPE *QueryInterface)(void *, REFIID, void **);
    ULONG(STDMETHODCALLTYPE *AddRef)(void *);
    ULONG(STDMETHODCALLTYPE *Release)(void *);
    HRESULT(STDMETHODCALLTYPE *InterfaceSupportsErrorInfo)(void *, REFIID);
} SupportVtbl;
struct Probe {
    struct ProbeVtbl *vtbl;
    SupportVtbl *support;
    LONG refs;
    BSTR name;
};
static const GUID iid_probe = {0xd0025a13, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
static HRESULT STDMETHODCALLTYPE p_query(Probe *t, REFIID i, void **o) {
    if (IsEqualGUID(i, &iid_supporterrorinfo)) {
        *o = &t->support;
    } else if (IsEqualGUID(i, &iid_unknown) || IsEqualGUID(i, &iid_dispatch) || IsEqualGUID(i, &iid_probe)) {
        *o = t;
    } else {
        *o = NULL;
        return E_NOINTERFACE;
    }
    t->refs++;
    return S_OK;
}
static ULONG STDMETHODCALLTYPE p_add(Probe *t) { return (ULONG)++t->refs; }
static ULONG STDMETHODCALLTYPE p_release(Probe *t) { return (ULONG)--t->refs; }
static HRESULT STDMETHODCALLTYPE p_count(Probe *t, UINT *n) {
    (void)t;
    *n = 1;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_typeinfo(Probe *t, UINT i, LCID l, ITypeInfo **o) {
    (void)t;
    (void)i;
    (void)l;
    ITypeInfo_AddRef(probe_info);
    *o = probe_info;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_ids(Probe *t, REFIID r, LPOLESTR *n, UINT c, LCID l, DISPID *ids) {
    (void)t;
    (void)r;
    (void)l;
    return DispGetIDsOfNames(probe_info, n, c, ids);
}
static HRESULT STDMETHODCALLTYPE p_invoke(Probe *t, DISPID id, REFIID r, LCID l, WORD f, DISPPARAMS *dp,
                                          VARIANT *res, EXCEPINFO *ei, UINT *arg) {
    (void)r;
    (void)l;
    return DispInvoke(t, probe_info, id, f, dp, res, ei, arg);
}
static HRESULT STDMETHODCALLTYPE p_get_name(Probe *t, BSTR *v) {
    *v = SysAllocString(t->name);
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_put_name(Probe *t, BSTR v) {
    SysFreeString(t->name);
    t->name = SysAllocString(v);
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_addn(Probe *t, LONG a, LONG b, LONG *s) {
    (void)t;
    *s = a + b;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_echo(Probe *t, VARIANT v, VARIANT *c) {
    (void)t;
    VariantInit(c);
    return VariantCopy(c, &v);
}
static HRESULT STDMETHODCALLTYPE p_scale(Probe *t, double x, double f, double *r) {
    (void)t;
    *r = x * f;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_fail(Probe *t) {
    ICreateErrorInfo *c = NULL;
    IErrorInfo *e = NULL;
    (void)t;
    CreateErrorInfo(&c);
    ICreateErrorInfo_SetDescription(c, L"The probe failed on purpose");
    ICreateErrorInfo_SetSource(c, L"Doors.Probe");
    ICreateErrorInfo_SetGUID(c, &iid_probe);
    ICreateErrorInfo_QueryInterface(c, &iid_errorinfo, (void **)&e);
    SetErrorInfo(0, e);
    IErrorInfo_Release(e);
    ICreateErrorInfo_Release(c);
    return E_FAIL;
}
static HRESULT STDMETHODCALLTYPE p_join(Probe *t, SAFEARRAY *parts, BSTR *j) {
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
static HRESULT STDMETHODCALLTYPE p_color(Probe *t, LONG *c) {
    (void)t;
    *c = 2;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE p_countn(Probe *t, VARIANT start, LONG *n) {
    (void)t;
    *n = V_VT(&start) == VT_ERROR ? -1 : V_VT(&start);
    return S_OK;
}
static struct ProbeVtbl probe_vtbl = {p_query, p_add, p_release, p_count, p_typeinfo, p_ids, p_invoke,
                                      p_get_name, p_put_name, p_addn, p_echo, p_scale, p_fail, p_join,
                                      p_color, p_countn};
static HRESULT STDMETHODCALLTYPE s_query(void *t, REFIID i, void **o) {
    return p_query((Probe *)((char *)t - sizeof(void *)), i, o);
}
static ULONG STDMETHODCALLTYPE s_add(void *t) { return p_add((Probe *)((char *)t - sizeof(void *))); }
static ULONG STDMETHODCALLTYPE s_release(void *t) { return p_release((Probe *)((char *)t - sizeof(void *))); }
static int no_support; /* ISupportErrorInfo answers S_FALSE */
static HRESULT STDMETHODCALLTYPE s_supports(void *t, REFIID i) {
    (void)t;
    return !no_support && IsEqualGUID(i, &iid_probe) ? S_OK : S_FALSE;
}
static SupportVtbl support_vtbl = {s_query, s_add, s_release, s_supports};

static void call(IDispatch *d, const char *label, DISPID id, WORD flags, VARIANT *args, UINT n, DISPID *named,
                 UINT nnamed) {
    DISPPARAMS dp;
    VARIANT res;
    EXCEPINFO ei;
    UINT arg = 99;
    HRESULT h;
    dp.rgvarg = args;
    dp.cArgs = n;
    dp.rgdispidNamedArgs = named;
    dp.cNamedArgs = nnamed;
    VariantInit(&res);
    memset(&ei, 0, sizeof ei);
    h = IDispatch_Invoke(d, id, &iid_null, 0x409, flags, &dp, &res, &ei, &arg);
    put("invoke ");
    put(label);
    put(" ");
    hex((unsigned)h);
    put(" ");
    show(&res);
    if (h == DISP_E_EXCEPTION) {
        put(" scode=");
        hex((unsigned)ei.scode);
        put(" code=");
        num(ei.wCode);
        put(" ");
        bput(ei.bstrSource);
        put(" ");
        bput(ei.bstrDescription);
        SysFreeString(ei.bstrSource);
        SysFreeString(ei.bstrDescription);
        SysFreeString(ei.bstrHelpFile);
    }
    if (h == DISP_E_TYPEMISMATCH || h == DISP_E_PARAMNOTFOUND) {
        put(" arg=");
        num(arg);
    }
    nl();
    VariantClear(&res);
}
static void dispatch(void) {
    Probe obj;
    IDispatch *d;
    VARIANT a[3];
    DISPID put_id = DISPID_PROPERTYPUT, ids[3];
    LPOLESTR nm[3];
    HRESULT h;
    if (!probe_info) return;
    obj.vtbl = &probe_vtbl;
    obj.support = &support_vtbl;
    obj.refs = 1;
    obj.name = SysAllocString(L"first");
    d = (IDispatch *)&obj;
    nm[0] = L"ADD";
    nm[1] = L"b";
    nm[2] = L"a";
    h = IDispatch_GetIDsOfNames(d, &iid_null, nm, 3, 0x409, ids);
    put("ids add ");
    hex((unsigned)h);
    put(" ");
    num(ids[0]);
    put(" ");
    num(ids[1]);
    put(" ");
    num(ids[2]);
    nm[0] = L"Nope";
    nm[1] = L"a";
    h = IDispatch_GetIDsOfNames(d, &iid_null, nm, 2, 0x409, ids);
    put(" unknown ");
    hex((unsigned)h);
    put(" ");
    num(ids[0]);
    put(" ");
    num(ids[1]);
    nl();
    call(d, "get-name", 1, DISPATCH_PROPERTYGET, NULL, 0, NULL, 0);
    V_VT(&a[0]) = VT_BSTR;
    V_BSTR(&a[0]) = SysAllocString(L"second");
    call(d, "put-name", 1, DISPATCH_PROPERTYPUT, a, 1, &put_id, 1);
    call(d, "put-name-unnamed", 1, DISPATCH_PROPERTYPUT, a, 1, NULL, 0);
    VariantClear(&a[0]);
    call(d, "get-name-again", 1, DISPATCH_PROPERTYGET | DISPATCH_METHOD, NULL, 0, NULL, 0);
    /* Arguments come last first: Add(2, 3). */
    V_VT(&a[1]) = VT_I4;
    V_I4(&a[1]) = 2;
    V_VT(&a[0]) = VT_BSTR;
    V_BSTR(&a[0]) = SysAllocString(L"3");
    call(d, "add-coerce", 2, DISPATCH_METHOD, a, 2, NULL, 0);
    VariantClear(&a[0]);
    V_VT(&a[0]) = VT_BSTR;
    V_BSTR(&a[0]) = SysAllocString(L"abc");
    call(d, "add-mismatch", 2, DISPATCH_METHOD, a, 2, NULL, 0);
    VariantClear(&a[0]);
    call(d, "add-one", 2, DISPATCH_METHOD, a + 1, 1, NULL, 0);
    /* Named: b=10 (dispid 1 among the parameters), a=5. */
    V_VT(&a[0]) = VT_I4;
    V_I4(&a[0]) = 10;
    V_VT(&a[1]) = VT_I4;
    V_I4(&a[1]) = 5;
    ids[0] = 1;
    ids[1] = 0;
    call(d, "add-named", 2, DISPATCH_METHOD, a, 2, ids, 2);
    V_VT(&a[0]) = VT_R8;
    V_R8(&a[0]) = 1.5;
    call(d, "scale-default", 4, DISPATCH_METHOD, a, 1, NULL, 0);
    V_VT(&a[1]) = VT_R8;
    V_R8(&a[1]) = 1.5;
    V_VT(&a[0]) = VT_I4;
    V_I4(&a[0]) = 4;
    call(d, "scale-two", 4, DISPATCH_METHOD, a, 2, NULL, 0);
    call(d, "fail", 5, DISPATCH_METHOD, NULL, 0, NULL, 0);
    no_support = 1;
    call(d, "fail-nosupport", 5, DISPATCH_METHOD, NULL, 0, NULL, 0);
    no_support = 0;
    {
        IErrorInfo *left = NULL;
        put("leftover ");
        hex((unsigned)GetErrorInfo(0, &left));
        nl();
        if (left) IErrorInfo_Release(left);
    }
    V_VT(&a[0]) = VT_DATE;
    V_DATE(&a[0]) = 37623.5;
    call(d, "echo", 3, DISPATCH_METHOD, a, 1, NULL, 0);
    call(d, "color", 7, DISPATCH_PROPERTYGET, NULL, 0, NULL, 0);
    call(d, "count-missing", 8, DISPATCH_METHOD, NULL, 0, NULL, 0);
    V_VT(&a[0]) = VT_ERROR;
    V_ERROR(&a[0]) = DISP_E_PARAMNOTFOUND;
    call(d, "count-error", 8, DISPATCH_METHOD, a, 1, NULL, 0);
    V_VT(&a[0]) = VT_I2;
    V_I2(&a[0]) = 7;
    call(d, "count-given", 8, DISPATCH_METHOD, a, 1, NULL, 0);
    {
        SAFEARRAY *sa = SafeArrayCreateVector(VT_BSTR, 0, 3);
        LONG i;
        static const wchar_t *words[] = {L"Macro", L"hard", L"Doors"};
        for (i = 0; i < 3; i++) {
            BSTR w = SysAllocString(words[i]);
            SafeArrayPutElement(sa, &i, w);
            SysFreeString(w);
        }
        V_VT(&a[0]) = VT_ARRAY | VT_BSTR;
        V_ARRAY(&a[0]) = sa;
        call(d, "join", 6, DISPATCH_METHOD, a, 1, NULL, 0);
        SafeArrayDestroy(sa);
    }
    call(d, "nomember", 99, DISPATCH_METHOD, NULL, 0, NULL, 0);
    call(d, "method-as-put", 2, DISPATCH_PROPERTYPUT, NULL, 0, NULL, 0);
    put("refs ");
    num(obj.refs);
    nl();
    /* CreateStdDispatch over the vtable interface. */
    {
        IUnknown *std = NULL;
        IDispatch *sd = NULL;
        UINT count = 9;
        h = CreateStdDispatch(NULL, &obj, probe_info, &std);
        put("std create ");
        hex((unsigned)h);
        IUnknown_QueryInterface(std, &iid_dispatch, (void **)&sd);
        IDispatch_GetTypeInfoCount(sd, &count);
        put(" count=");
        num(count);
        nl();
        V_VT(&a[1]) = VT_I4;
        V_I4(&a[1]) = 40;
        V_VT(&a[0]) = VT_I4;
        V_I4(&a[0]) = 2;
        call(sd, "std-add", 2, DISPATCH_METHOD, a, 2, NULL, 0);
        call(sd, "std-fail", 5, DISPATCH_METHOD, NULL, 0, NULL, 0);
        IDispatch_Release(sd);
        IUnknown_Release(std);
    }
    /* DispGetParam. */
    {
        DISPPARAMS dp;
        VARIANT r;
        UINT arg = 9;
        V_VT(&a[1]) = VT_BSTR;
        V_BSTR(&a[1]) = SysAllocString(L"12");
        V_VT(&a[0]) = VT_R8;
        V_R8(&a[0]) = 2.5;
        dp.rgvarg = a;
        dp.cArgs = 2;
        dp.rgdispidNamedArgs = NULL;
        dp.cNamedArgs = 0;
        VariantInit(&r);
        put("param 0 ");
        hex((unsigned)DispGetParam(&dp, 0, VT_I4, &r, &arg));
        put(" ");
        show(&r);
        VariantClear(&r);
        put(" 1 ");
        hex((unsigned)DispGetParam(&dp, 1, VT_I4, &r, &arg));
        put(" ");
        show(&r);
        VariantClear(&r);
        put(" 2 ");
        hex((unsigned)DispGetParam(&dp, 2, VT_I4, &r, &arg));
        put(" bstr-bad ");
        V_VT(&a[1]) = VT_BSTR;
        hex((unsigned)DispGetParam(&dp, 0, VT_DISPATCH, &r, &arg));
        put(" arg=");
        num(arg);
        nl();
        VariantClear(&a[1]);
    }
    SysFreeString(obj.name);
}

/* `dump <tlb>`: every type of any type library (no checks). */
static void dump(const wchar_t *path) {
    ITypeLib *tl = NULL;
    TLIBATTR *la = NULL;
    BSTR name = NULL, doc = NULL, file = NULL;
    DWORD ctx = 0;
    UINT i, n;
    HRESULT h = LoadTypeLibEx(path, REGKIND_NONE, &tl);
    put("load ");
    hex((unsigned)h);
    nl();
    if (h != S_OK) return;
    ITypeLib_GetLibAttr(tl, &la);
    put("lib guid=");
    guid(&la->guid);
    put(" lcid=");
    num(la->lcid);
    put(" sys=");
    num(la->syskind);
    put(" ver=");
    num(la->wMajorVerNum);
    put(".");
    num(la->wMinorVerNum);
    put(" flags=");
    hex(la->wLibFlags);
    ITypeLib_ReleaseTLibAttr(tl, la);
    ITypeLib_GetDocumentation(tl, -1, &name, &doc, &ctx, &file);
    put(" ");
    bput(name);
    put(" ");
    bput(doc);
    put(" ctx=");
    num(ctx);
    put(" file=");
    bput(file);
    n = ITypeLib_GetTypeInfoCount(tl);
    put(" count=");
    num(n);
    nl();
    for (i = 0; i < n; i++) {
        ITypeInfo *ti = NULL, *vt = NULL;
        TYPEATTR *ta = NULL;
        HREFTYPE href;
        char label[16] = "t";
        int k = 1;
        if (i >= 10) label[k++] = (char)('0' + i / 10);
        label[k++] = (char)('0' + i % 10);
        label[k] = 0;
        ITypeLib_GetTypeInfo(tl, i, &ti);
        typeinfo(ti, label);
        ITypeInfo_GetTypeAttr(ti, &ta);
        if (ta->typekind == TKIND_DISPATCH && (ta->wTypeFlags & TYPEFLAG_FDUAL) &&
            ITypeInfo_GetRefTypeOfImplType(ti, (UINT)-1, &href) == S_OK &&
            ITypeInfo_GetRefTypeInfo(ti, href, &vt) == S_OK) {
            label[k] = 'v';
            label[k + 1] = 0;
            typeinfo(vt, label);
            ITypeInfo_Release(vt);
        }
        ITypeInfo_ReleaseTypeAttr(ti, ta);
        ITypeInfo_Release(ti);
    }
    ITypeLib_Release(tl);
}

/* ---- registration (guest only: it writes the registry) ------------------------------------ */
static const wchar_t *library_path; /* the type library the command line names */
static const GUID probe_lib = {0xd0025a10, 0xc0a1, 0x4e5b, {0x8f, 0x00, 0x43, 0x6f, 0x6d, 0x50, 0x72, 0x6f}};
static const GUID stdole_lib = {0x00020430, 0, 0, {0xC0, 0, 0, 0, 0, 0, 0, 0x46}};
static int same_text(const wchar_t *a, const wchar_t *b) {
    while (*a && *b) {
        wchar_t x = *a >= L'A' && *a <= L'Z' ? *a + 32 : *a, y = *b >= L'A' && *b <= L'Z' ? *b + 32 : *b;
        if (x != y) return 0;
        a++;
        b++;
    }
    return *a == *b;
}
/* RegisterTypeLibForUser, QueryPathOfRegTypeLib, LoadRegTypeLib (the same
 * object LoadTypeLib gave), the Interface key with the automation proxy,
 * UnRegisterTypeLibForUser, and stdole2 by GUID. Prints the first failed
 * check's number (0: all passed). */
static void registration(void) {
    ITypeLib *tl = NULL, *again = NULL, *std = NULL;
    BSTR path = NULL;
    wchar_t value[64];
    DWORD size = sizeof value;
    int failed = 0, check = 0;
#define CHECK(c)                       \
    do {                               \
        check++;                       \
        if (!failed && !(c)) failed = check; \
    } while (0)
    CHECK(LoadTypeLibEx(library_path, REGKIND_NONE, &tl) == S_OK);
    CHECK(QueryPathOfRegTypeLib(&probe_lib, 1, 2, 0, &path) == TYPE_E_LIBNOTREGISTERED);
    CHECK(tl && RegisterTypeLibForUser(tl, (LPOLESTR)library_path, NULL) == S_OK);
    CHECK(QueryPathOfRegTypeLib(&probe_lib, 1, 2, 0, &path) == S_OK && path && same_text(path, library_path));
    SysFreeString(path);
    path = NULL;
    CHECK(LoadRegTypeLib(&probe_lib, 1, 0, 0x409, &again) == S_OK && again == tl);
    CHECK(RegGetValueW(HKEY_CURRENT_USER,
                       L"Software\\Classes\\Interface\\{D0025A13-C0A1-4E5B-8F00-436F6D50726F}\\ProxyStubClsid32",
                       NULL, RRF_RT_REG_SZ, NULL, value, &size) == ERROR_SUCCESS &&
          same_text(value, L"{00020424-0000-0000-C000-000000000046}"));
    CHECK(UnRegisterTypeLibForUser(&probe_lib, 1, 2, 0, SYS_WIN64) == S_OK);
    CHECK(QueryPathOfRegTypeLib(&probe_lib, 1, 2, 0, &path) == TYPE_E_LIBNOTREGISTERED);
    CHECK(LoadRegTypeLib(&stdole_lib, 2, 0, 0, &std) == S_OK && std && ITypeLib_GetTypeInfoCount(std) == 42);
#undef CHECK
    if (std) ITypeLib_Release(std);
    if (again) ITypeLib_Release(again);
    if (tl) ITypeLib_Release(tl);
    put("registration ");
    num(failed);
    put("\r\n");
}

/* ---- steps -------------------------------------------------------------------------------- */
/* What Windows 11 26H2 printed for each step (oleautprobe.exe raw). */
static const char *const expected[] = {
#include "oleautprobe.expected.h"
    /* registration (guest only): every check passes */
    , "registration 0\r\n"
};

static void typelib_step(void) { typelib(library_path); }
static const struct {
    const char *name;
    void (*run)(void);
} steps[] = {
    {"bstr", bstrs},          {"variant", variants},   {"safearray", arrays},  {"dates", dates},
    {"errorinfo", errors},    {"typelib", typelib_step}, {"dispatch", dispatch}, {"registration", registration},
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[400];
    int n = line_length(s), k;
    if (n > (int)sizeof b - 1) n = sizeof b - 1;
    for (k = 0; k < n; k++) b[k] = s[k];
    b[n] = 0;
    put("  ");
    put(label);
    put(": ");
    put(b);
    put("\r\n");
}
/* The first line where `got` and `want` differ (0: none), counting lines. */
static int compare(const char *got, const char *want, int *lines, const char **g, const char **w) {
    int line = 1;
    *lines = 0;
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
        (*lines)++;
        line++;
        continue;
    differ:
        *g = got;
        *w = want;
        return line;
    }
}

void entry(void) {
    int argc = 0, i, failed = 0, count;
    LPWSTR *argv;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (argc >= 3 && argv[1][0] == L'd') {
        dump(argv[2]);
        ExitProcess(0);
    }
    if (argc < 3) {
        put("usage: oleautprobe.exe check|guest|raw doorsprobe.tlb | dump any.tlb\r\n");
        ExitProcess(2);
    }
    library_path = argv[2];
    /* `guest` adds registration, which writes the registry. */
    count = (int)(sizeof steps / sizeof steps[0]) - (argv[1][0] == L'g' ? 0 : 1);
    for (i = 0; i < count; i++) {
        int lines, at;
        const char *g = 0, *w = 0;
        if (argv[1][0] == L'r') {
            put("== ");
            put(steps[i].name);
            put("\r\n");
            steps[i].run();
            continue;
        }
        captured_length = 0;
        captured[0] = 0;
        capturing = 1;
        steps[i].run();
        capturing = 0;
        at = compare(captured, expected[i], &lines, &g, &w);
        put("OLEAUT-A ");
        num(i + 1);
        put(" ");
        put(steps[i].name);
        if (at == 0) {
            put(" ok ");
            num(lines);
            put("\r\n");
        } else {
            put(" FAILED line ");
            num(at);
            put("\r\n");
            put_line("got", g);
            put_line("expected", w);
            failed = 1;
        }
    }
    if (probe_info) ITypeInfo_Release(probe_info);
    if (probe_dispinfo) ITypeInfo_Release(probe_dispinfo);
    if (argv[1][0] == L'r') ExitProcess(0);
    put(failed ? "OLEAUT-A FAILED\r\n" : "OLEAUT-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

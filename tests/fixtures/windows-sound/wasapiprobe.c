/* WASAPI-A probe (P15 M4): an ordinary Windows console program (Windows
 * SDK, no C runtime) that uses the Core Audio APIs through COM:
 *
 *   wasapiprobe.exe [reference | guest]
 *
 * MMDeviceEnumerator, the default render endpoint and its properties,
 * IAudioClient in shared mode (mix format, periods, format support, buffer
 * sizes, state errors), IAudioRenderClient, IAudioClock, event-driven
 * streams, ISimpleAudioVolume, IAudioEndpointVolume and endpoint
 * notifications. "reference" runs on the owner's Windows PC: it renders
 * silence only, reads volumes without setting them and never opens the
 * microphone. "guest" also sets volumes and captures. Device-dependent
 * numbers are printed as "WASAPI-RAW name value" lines.
 *
 * One "WASAPI-A <step> <name> ok|FAILED <detail>" line per step; exit 0
 * only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <endpointvolume.h>
#include <functiondiscoverykeys_devpkey.h>
#include <mmsystem.h>
#include <intrin.h>

int _fltused = 0;
#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
    return destination;
}

static const GUID clsid_enumerator = {0xbcde0395, 0xe52f, 0x467c, {0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e}};
static const GUID iid_enumerator = {0xa95664d2, 0x9614, 0x4f35, {0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6}};
static const GUID iid_endpoint = {0x1be09788, 0x6894, 0x4089, {0x85, 0x86, 0x9a, 0x2a, 0x6c, 0x26, 0x5a, 0xc5}};
static const GUID iid_client = {0x1cb9ad4c, 0xdbfa, 0x4c32, {0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2}};
static const GUID iid_render = {0xf294acfc, 0x3146, 0x4483, {0xa7, 0xbf, 0xad, 0xdc, 0xa7, 0xc2, 0x60, 0xe2}};
static const GUID iid_capture = {0xc8adbd64, 0xe71e, 0x48a0, {0xa4, 0xde, 0x18, 0x5c, 0x39, 0x5c, 0xd3, 0x17}};
static const GUID iid_clock = {0xcd63314f, 0x3fba, 0x4a1b, {0x81, 0x2c, 0xef, 0x96, 0x35, 0x87, 0x28, 0xe7}};
static const GUID iid_simple = {0x87ce5498, 0x68d6, 0x44e5, {0x92, 0x15, 0x6d, 0xa4, 0x7e, 0xf8, 0x83, 0xd8}};
static const GUID iid_volume = {0x5cdf2c82, 0x841e, 0x4546, {0x97, 0x22, 0x0c, 0xf7, 0x40, 0x78, 0x22, 0x9a}};
static const GUID iid_bogus = {0xd0025d99, 0x0000, 0x0000, {0, 0, 0, 0, 0, 0, 0, 1}};
static const GUID subtype_float = {0x00000003, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};

#define E_NOTFOUND_ 0x80070490

static HANDLE out;
static int guest;
static int length(const char *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static void put(const char *s) {
    DWORD written;
    WriteFile(out, s, (DWORD)length(s), &written, NULL);
}
static void number(LONGLONG v) {
    char b[32];
    int i = 31;
    ULONGLONG u = v < 0 ? (ULONGLONG)(-v) : (ULONGLONG)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u && i > 1);
    if (v < 0) b[--i] = '-';
    put(b + i);
}
static void hex(ULONG v) {
    char b[11];
    b[0] = '0';
    b[1] = 'x';
    for (int i = 0; i < 8; i++) b[2 + i] = "0123456789abcdef"[(v >> (28 - 4 * i)) & 15];
    b[10] = 0;
    put(b);
}
static void raw(const char *name, LONGLONG v) {
    put("WASAPI-RAW ");
    put(name);
    put(" ");
    number(v);
    put("\r\n");
}
static void raw_hr(const char *name, HRESULT h) {
    put("WASAPI-RAW ");
    put(name);
    put(" ");
    hex((ULONG)h);
    put("\r\n");
}
static void raw_text(const char *name, const wchar_t *w) {
    char b[160];
    int i = 0;
    for (; w && w[i] && i < 159; i++) b[i] = w[i] < 128 ? (char)w[i] : '?';
    b[i] = 0;
    put("WASAPI-RAW ");
    put(name);
    put(" ");
    put(b);
    put("\r\n");
}
static int failed_step, check;
static HRESULT last;
static void step(int id, const char *name, int failed) {
    put("WASAPI-A ");
    number(id);
    put(" ");
    put(name);
    if (failed) {
        put(" FAILED check ");
        number(failed);
        put(" hr=");
        hex((ULONG)last);
        if (!failed_step) failed_step = id;
    } else {
        put(" ok ");
        number(check);
    }
    put("\r\n");
}
#define EXPECT(c)                     \
    do {                              \
        check++;                      \
        if (!(c)) return check;       \
    } while (0)
#define CALL(e, want)                 \
    do {                              \
        last = (e);                   \
        EXPECT(last == (HRESULT)(want)); \
    } while (0)
static int prefix(const wchar_t *s, const wchar_t *p) {
    while (*p)
        if (*s++ != *p++) return 0;
    return 1;
}
static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static int wide_length(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static int same_guid(const GUID *a, const GUID *b) {
    const unsigned char *x = (const unsigned char *)a, *y = (const unsigned char *)b;
    for (int i = 0; i < 16; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}
static void pcm(WAVEFORMATEX *f, DWORD rate, WORD channels, WORD bits) {
    f->wFormatTag = WAVE_FORMAT_PCM;
    f->nChannels = channels;
    f->nSamplesPerSec = rate;
    f->wBitsPerSample = bits;
    f->nBlockAlign = (WORD)(channels * bits / 8);
    f->nAvgBytesPerSec = rate * f->nBlockAlign;
    f->cbSize = 0;
}

/* ---- notifications ------------------------------------------------------------------------ */
static ULONG STDMETHODCALLTYPE n_add(IMMNotificationClient *t) {
    (void)t;
    return 2;
}
static HRESULT STDMETHODCALLTYPE n_query(IMMNotificationClient *t, REFIID i, void **o) {
    (void)i;
    *o = t;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE n_state(IMMNotificationClient *t, LPCWSTR id, DWORD s) {
    (void)t;
    (void)id;
    (void)s;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE n_added(IMMNotificationClient *t, LPCWSTR id) {
    (void)t;
    (void)id;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE n_default(IMMNotificationClient *t, EDataFlow f, ERole r, LPCWSTR id) {
    (void)t;
    (void)f;
    (void)r;
    (void)id;
    return S_OK;
}
static HRESULT STDMETHODCALLTYPE n_property(IMMNotificationClient *t, LPCWSTR id, const PROPERTYKEY k) {
    (void)t;
    (void)id;
    (void)k;
    return S_OK;
}
static IMMNotificationClientVtbl notify_vtbl = {n_query, n_add, n_add, n_state, n_added, n_added, n_default, n_property};
static IMMNotificationClient notify = {&notify_vtbl};

/* ---- steps -------------------------------------------------------------------------------- */
static IMMDeviceEnumerator *enumerator;
static IMMDevice *speaker;
static IAudioClient *client;
static WAVEFORMATEX *mix;
static UINT32 frames;
static wchar_t *speaker_id;

static int enumerator_step(void) {
    IUnknown *outer = (IUnknown *)&notify, *p = NULL;
    /* Windows 11 answers REGDB_E_CLASSNOTREG here, not CLASS_E_NOAGGREGATION. */
    last = CoCreateInstance(&clsid_enumerator, outer, CLSCTX_ALL, &iid_enumerator, (void **)&p);
    raw_hr("aggregate", last);
    EXPECT(last != S_OK && p == NULL);
    CALL(CoCreateInstance(&clsid_enumerator, NULL, CLSCTX_ALL, &iid_enumerator, (void **)&enumerator), S_OK);
    return 0;
}
static int default_step(void) {
    IMMDevice *again = NULL, *bad = NULL;
    IMMEndpoint *endpoint = NULL;
    wchar_t *id = NULL;
    DWORD state = 0;
    EDataFlow flow = eAll;
    CALL(IMMDeviceEnumerator_GetDefaultAudioEndpoint(enumerator, eRender, eConsole, &speaker), S_OK);
    CALL(IMMDevice_GetId(speaker, &speaker_id), S_OK);
    raw_text("render-id", speaker_id);
    EXPECT(prefix(speaker_id, L"{0.0.0.00000000}.{") && wide_length(speaker_id) == 55);
    CALL(IMMDevice_GetState(speaker, &state), S_OK);
    EXPECT(state == DEVICE_STATE_ACTIVE);
    CALL(IMMDevice_QueryInterface(speaker, &iid_endpoint, (void **)&endpoint), S_OK);
    CALL(IMMEndpoint_GetDataFlow(endpoint, &flow), S_OK);
    EXPECT(flow == eRender);
    IMMEndpoint_Release(endpoint);
    CALL(IMMDeviceEnumerator_GetDefaultAudioEndpoint(enumerator, eRender, eMultimedia, &again), S_OK);
    CALL(IMMDevice_GetId(again, &id), S_OK);
    EXPECT(wide_equal(id, speaker_id));
    CoTaskMemFree(id);
    IMMDevice_Release(again);
    CALL(IMMDeviceEnumerator_GetDevice(enumerator, speaker_id, &again), S_OK);
    IMMDevice_Release(again);
    CALL(IMMDeviceEnumerator_GetDevice(enumerator, L"{0.0.0.00000000}.{d0025d99-0000-0000-0000-000000000001}", &bad),
         E_NOTFOUND_);
    CALL(IMMDeviceEnumerator_GetDefaultAudioEndpoint(enumerator, eAll, eConsole, &bad), E_INVALIDARG);
    CALL(IMMDeviceEnumerator_GetDefaultAudioEndpoint(enumerator, eRender, eConsole, NULL), E_POINTER);
    return 0;
}
static int collection_step(void) {
    IMMDeviceCollection *c = NULL;
    IMMDevice *d = NULL;
    UINT n = 0, found = 0;
    CALL(IMMDeviceEnumerator_EnumAudioEndpoints(enumerator, eRender, DEVICE_STATE_ACTIVE, &c), S_OK);
    CALL(IMMDeviceCollection_GetCount(c, &n), S_OK);
    raw("render-endpoints", n);
    EXPECT(n >= 1);
    for (UINT i = 0; i < n; i++) {
        wchar_t *id = NULL;
        CALL(IMMDeviceCollection_Item(c, i, &d), S_OK);
        CALL(IMMDevice_GetId(d, &id), S_OK);
        if (wide_equal(id, speaker_id)) found = 1;
        CoTaskMemFree(id);
        IMMDevice_Release(d);
    }
    EXPECT(found);
    CALL(IMMDeviceCollection_Item(c, n, &d), E_INVALIDARG);
    IMMDeviceCollection_Release(c);
    CALL(IMMDeviceEnumerator_EnumAudioEndpoints(enumerator, eRender, 0x10, &c), E_INVALIDARG);
    return 0;
}
static int properties_step(void) {
    IPropertyStore *store = NULL;
    PROPVARIANT v;
    DWORD n = 0;
    CALL(IMMDevice_OpenPropertyStore(speaker, STGM_READ, &store), S_OK);
    CALL(IPropertyStore_GetCount(store, &n), S_OK);
    EXPECT(n > 0);
    PropVariantInit(&v);
    CALL(IPropertyStore_GetValue(store, &PKEY_Device_FriendlyName, &v), S_OK);
    EXPECT(v.vt == VT_LPWSTR && v.pwszVal && v.pwszVal[0]);
    raw_text("friendly-name", v.pwszVal);
    PropVariantClear(&v);
    CALL(IPropertyStore_GetValue(store, &PKEY_AudioEngine_DeviceFormat, &v), S_OK);
    EXPECT(v.vt == VT_BLOB && v.blob.cbSize >= 18);
    PropVariantClear(&v);
    PropVariantInit(&v);
    v.vt = VT_UI4;
    CALL(IPropertyStore_SetValue(store, &PKEY_Device_FriendlyName, &v), E_ACCESSDENIED);
    IPropertyStore_Release(store);
    return 0;
}
static int mix_step(void) {
    IUnknown *p = NULL;
    const WAVEFORMATEXTENSIBLE *x;
    CALL(IMMDevice_Activate(speaker, &iid_bogus, CLSCTX_ALL, NULL, (void **)&p), E_NOINTERFACE);
    CALL(IMMDevice_Activate(speaker, &iid_client, CLSCTX_ALL, NULL, (void **)&client), S_OK);
    CALL(IAudioClient_GetMixFormat(client, &mix), S_OK);
    x = (const WAVEFORMATEXTENSIBLE *)mix;
    raw("mix-tag", mix->wFormatTag);
    raw("mix-channels", mix->nChannels);
    raw("mix-rate", mix->nSamplesPerSec);
    raw("mix-bits", mix->wBitsPerSample);
    raw("mix-align", mix->nBlockAlign);
    raw("mix-average", mix->nAvgBytesPerSec);
    raw("mix-cbsize", mix->cbSize);
    raw("mix-valid", x->Samples.wValidBitsPerSample);
    raw("mix-mask", x->dwChannelMask);
    EXPECT(mix->wFormatTag == WAVE_FORMAT_EXTENSIBLE && mix->cbSize == 22 && mix->wBitsPerSample == 32);
    EXPECT(same_guid(&x->SubFormat, &subtype_float));
    EXPECT(mix->nAvgBytesPerSec == mix->nSamplesPerSec * mix->nBlockAlign);
    return 0;
}
static int format_step(void) {
    REFERENCE_TIME period = 0, minimum = 0;
    WAVEFORMATEX f, *closest = (WAVEFORMATEX *)1;
    UINT32 n = 0;
    CALL(IAudioClient_GetDevicePeriod(client, &period, &minimum), S_OK);
    raw("period-default", period);
    raw("period-minimum", minimum);
    EXPECT(period == 100000 && minimum > 0 && minimum <= period);
    CALL(IAudioClient_IsFormatSupported(client, AUDCLNT_SHAREMODE_SHARED, mix, &closest), S_OK);
    EXPECT(closest == NULL);
    pcm(&f, mix->nSamplesPerSec, mix->nChannels, 16);
    last = IAudioClient_IsFormatSupported(client, AUDCLNT_SHAREMODE_SHARED, &f, &closest);
    raw_hr("support-pcm16", last);
    if (closest) CoTaskMemFree(closest);
    pcm(&f, 44100, 2, 16);
    closest = NULL;
    last = IAudioClient_IsFormatSupported(client, AUDCLNT_SHAREMODE_SHARED, &f, &closest);
    raw_hr("support-44100", last);
    raw("support-44100-closest", closest != NULL);
    if (closest) {
        raw("support-44100-closest-rate", closest->nSamplesPerSec);
        CoTaskMemFree(closest);
    }
    pcm(&f, mix->nSamplesPerSec, 2, 12);
    closest = NULL;
    last = IAudioClient_IsFormatSupported(client, AUDCLNT_SHAREMODE_SHARED, &f, &closest);
    raw_hr("support-12bit", last);
    if (closest) CoTaskMemFree(closest);
    CALL(IAudioClient_IsFormatSupported(client, AUDCLNT_SHAREMODE_SHARED, NULL, &closest), E_POINTER);
    CALL(IAudioClient_GetBufferSize(client, &n), AUDCLNT_E_NOT_INITIALIZED);
    CALL(IAudioClient_GetCurrentPadding(client, &n), AUDCLNT_E_NOT_INITIALIZED);
    CALL(IAudioClient_Start(client), AUDCLNT_E_NOT_INITIALIZED);
    return 0;
}
static int initialize_step(void) {
    REFERENCE_TIME latency = 0;
    IAudioClient *other = NULL;
    WAVEFORMATEX f;
    UINT32 n = 0;
    CALL(IAudioClient_Initialize(client, AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, mix, NULL), S_OK);
    CALL(IAudioClient_GetBufferSize(client, &frames), S_OK);
    raw("buffer-default", frames);
    last = IAudioClient_GetStreamLatency(client, &latency);
    raw_hr("latency-hr", last);
    raw("latency", latency);
    EXPECT(frames > 0);
    /* 200 ms. */
    CALL(IMMDevice_Activate(speaker, &iid_client, CLSCTX_ALL, NULL, (void **)&other), S_OK);
    CALL(IAudioClient_Initialize(other, AUDCLNT_SHAREMODE_SHARED, 0, 2000000, 0, mix, NULL), S_OK);
    CALL(IAudioClient_Initialize(other, AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, mix, NULL), AUDCLNT_E_ALREADY_INITIALIZED);
    CALL(IAudioClient_GetBufferSize(other, &n), S_OK);
    raw("buffer-200ms", n);
    EXPECT(n >= mix->nSamplesPerSec / 5);
    IAudioClient_Release(other);
    /* 44.1 kHz PCM through the engine's converter. */
    pcm(&f, 44100, 2, 16);
    CALL(IMMDevice_Activate(speaker, &iid_client, CLSCTX_ALL, NULL, (void **)&other), S_OK);
    CALL(IAudioClient_Initialize(other, AUDCLNT_SHAREMODE_SHARED,
                                 AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 0, 0,
                                 &f, NULL),
         S_OK);
    CALL(IAudioClient_GetBufferSize(other, &n), S_OK);
    raw("buffer-44100", n);
    IAudioClient_Release(other);
    return 0;
}
static IAudioRenderClient *render;
static IAudioClock *clock_;
static int render_step(void) {
    IUnknown *p = NULL;
    BYTE *data = NULL;
    UINT32 padding = 0;
    CALL(IAudioClient_GetService(client, &iid_bogus, (void **)&p), E_NOINTERFACE);
    CALL(IAudioClient_GetService(client, &iid_capture, (void **)&p), AUDCLNT_E_WRONG_ENDPOINT_TYPE);
    CALL(IAudioClient_GetService(client, &iid_render, (void **)&render), S_OK);
    CALL(IAudioRenderClient_GetBuffer(render, frames + 1, &data), AUDCLNT_E_BUFFER_TOO_LARGE);
    CALL(IAudioRenderClient_ReleaseBuffer(render, 1, 0), AUDCLNT_E_OUT_OF_ORDER);
    CALL(IAudioRenderClient_GetBuffer(render, frames, &data), S_OK);
    EXPECT(data != NULL);
    CALL(IAudioRenderClient_GetBuffer(render, 1, &data), AUDCLNT_E_OUT_OF_ORDER);
    CALL(IAudioRenderClient_ReleaseBuffer(render, frames + 1, 0), AUDCLNT_E_INVALID_SIZE);
    CALL(IAudioRenderClient_ReleaseBuffer(render, frames, AUDCLNT_BUFFERFLAGS_SILENT), S_OK);
    CALL(IAudioClient_GetCurrentPadding(client, &padding), S_OK);
    EXPECT(padding == frames);
    return 0;
}
static int clock_step(void) {
    UINT64 frequency = 0, position = 1, qpc = 0;
    DWORD characteristics = 0;
    CALL(IAudioClient_GetService(client, &iid_clock, (void **)&clock_), S_OK);
    CALL(IAudioClock_GetFrequency(clock_, &frequency), S_OK);
    raw("clock-frequency", (LONGLONG)frequency);
    EXPECT(frequency == mix->nAvgBytesPerSec);
    CALL(IAudioClock_GetPosition(clock_, &position, &qpc), S_OK);
    EXPECT(position == 0);
    CALL(IAudioClock_GetCharacteristics(clock_, &characteristics), S_OK);
    raw("clock-characteristics", characteristics);
    return 0;
}
static int run_step(void) {
    UINT32 padding = 0, written = 0;
    UINT64 position = 0, qpc = 0;
    BYTE *data;
    CALL(IAudioClient_Start(client), S_OK);
    CALL(IAudioClient_Start(client), AUDCLNT_E_NOT_STOPPED);
    for (int i = 0; i < 30; i++) {
        Sleep(10);
        CALL(IAudioClient_GetCurrentPadding(client, &padding), S_OK);
        if (padding < frames) {
            UINT32 free = frames - padding;
            CALL(IAudioRenderClient_GetBuffer(render, free, &data), S_OK);
            CALL(IAudioRenderClient_ReleaseBuffer(render, free, AUDCLNT_BUFFERFLAGS_SILENT), S_OK);
            written += free;
        }
    }
    raw("frames-refilled", written);
    EXPECT(written > 0);
    CALL(IAudioClock_GetPosition(clock_, &position, &qpc), S_OK);
    raw("position-300ms", (LONGLONG)position);
    EXPECT(position > 0 && qpc > 0);
    CALL(IAudioClient_Reset(client), AUDCLNT_E_NOT_STOPPED);
    CALL(IAudioClient_Stop(client), S_OK);
    CALL(IAudioClient_Stop(client), S_FALSE);
    CALL(IAudioClient_Reset(client), S_OK);
    CALL(IAudioClient_GetCurrentPadding(client, &padding), S_OK);
    EXPECT(padding == 0);
    CALL(IAudioClock_GetPosition(clock_, &position, &qpc), S_OK);
    EXPECT(position == 0);
    return 0;
}
static int event_step(void) {
    IAudioClient *e = NULL;
    IAudioRenderClient *r = NULL;
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    UINT32 n = 0, padding = 0;
    BYTE *data;
    int signals = 0;
    CALL(IAudioClient_SetEventHandle(client, event), AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED);
    CALL(IMMDevice_Activate(speaker, &iid_client, CLSCTX_ALL, NULL, (void **)&e), S_OK);
    CALL(IAudioClient_Initialize(e, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0, 0, mix, NULL), S_OK);
    CALL(IAudioClient_Start(e), AUDCLNT_E_EVENTHANDLE_NOT_SET);
    CALL(IAudioClient_SetEventHandle(e, NULL), E_INVALIDARG);
    CALL(IAudioClient_SetEventHandle(e, event), S_OK);
    CALL(IAudioClient_GetBufferSize(e, &n), S_OK);
    raw("buffer-event", n);
    CALL(IAudioClient_GetService(e, &iid_render, (void **)&r), S_OK);
    CALL(IAudioRenderClient_GetBuffer(r, n, &data), S_OK);
    CALL(IAudioRenderClient_ReleaseBuffer(r, n, AUDCLNT_BUFFERFLAGS_SILENT), S_OK);
    CALL(IAudioClient_Start(e), S_OK);
    for (int i = 0; i < 20; i++) {
        if (WaitForSingleObject(event, 500) != WAIT_OBJECT_0) break;
        signals++;
        CALL(IAudioClient_GetCurrentPadding(e, &padding), S_OK);
        if (padding < n) {
            CALL(IAudioRenderClient_GetBuffer(r, n - padding, &data), S_OK);
            CALL(IAudioRenderClient_ReleaseBuffer(r, n - padding, AUDCLNT_BUFFERFLAGS_SILENT), S_OK);
        }
    }
    raw("event-signals", signals);
    EXPECT(signals == 20);
    CALL(IAudioClient_Stop(e), S_OK);
    IAudioRenderClient_Release(r);
    IAudioClient_Release(e);
    CloseHandle(event);
    return 0;
}
static int session_step(void) {
    ISimpleAudioVolume *v = NULL;
    float level = 0;
    BOOL mute = TRUE;
    CALL(IAudioClient_GetService(client, &iid_simple, (void **)&v), S_OK);
    CALL(ISimpleAudioVolume_GetMasterVolume(v, &level), S_OK);
    raw("session-volume-milli", (LONGLONG)(level * 1000.0f + 0.5f));
    EXPECT(level > 0.0f && level <= 1.0f);
    CALL(ISimpleAudioVolume_GetMute(v, &mute), S_OK);
    EXPECT(mute == FALSE);
    CALL(ISimpleAudioVolume_SetMasterVolume(v, 1.5f, NULL), E_INVALIDARG);
    if (guest) {
        CALL(ISimpleAudioVolume_SetMasterVolume(v, 0.5f, NULL), S_OK);
        CALL(ISimpleAudioVolume_GetMasterVolume(v, &level), S_OK);
        EXPECT(level > 0.49f && level < 0.51f);
        CALL(ISimpleAudioVolume_SetMute(v, TRUE, NULL), S_OK);
        CALL(ISimpleAudioVolume_GetMute(v, &mute), S_OK);
        EXPECT(mute == TRUE);
        CALL(ISimpleAudioVolume_SetMute(v, FALSE, NULL), S_OK);
        CALL(ISimpleAudioVolume_SetMasterVolume(v, 1.0f, NULL), S_OK);
    }
    ISimpleAudioVolume_Release(v);
    return 0;
}
static int endpoint_step(void) {
    IAudioEndpointVolume *v = NULL;
    UINT channels = 0, stepping = 0, steps = 0;
    float level = -1, mindb = 0, maxdb = 0, incdb = 0;
    BOOL mute = TRUE;
    DWORD support = 0;
    CALL(IMMDevice_Activate(speaker, &iid_volume, CLSCTX_ALL, NULL, (void **)&v), S_OK);
    CALL(IAudioEndpointVolume_GetChannelCount(v, &channels), S_OK);
    raw("endpoint-channels", channels);
    EXPECT(channels == mix->nChannels);
    CALL(IAudioEndpointVolume_GetMasterVolumeLevelScalar(v, &level), S_OK);
    EXPECT(level >= 0.0f && level <= 1.0f);
    CALL(IAudioEndpointVolume_GetMute(v, &mute), S_OK);
    CALL(IAudioEndpointVolume_GetVolumeRange(v, &mindb, &maxdb, &incdb), S_OK);
    raw("range-min-centidb", (LONGLONG)(mindb * 100.0f));
    raw("range-max-centidb", (LONGLONG)(maxdb * 100.0f));
    raw("range-step-centidb", (LONGLONG)(incdb * 100.0f));
    CALL(IAudioEndpointVolume_GetVolumeStepInfo(v, &stepping, &steps), S_OK);
    raw("steps", steps);
    CALL(IAudioEndpointVolume_QueryHardwareSupport(v, &support), S_OK);
    raw("hardware-support", support);
    CALL(IAudioEndpointVolume_GetChannelVolumeLevelScalar(v, channels, &level), E_INVALIDARG);
    CALL(IAudioEndpointVolume_SetMasterVolumeLevelScalar(v, 1.5f, NULL), E_INVALIDARG);
    if (guest) {
        float before = 0;
        CALL(IAudioEndpointVolume_GetMasterVolumeLevelScalar(v, &before), S_OK);
        CALL(IAudioEndpointVolume_SetMasterVolumeLevelScalar(v, 0.25f, NULL), S_OK);
        CALL(IAudioEndpointVolume_GetMasterVolumeLevelScalar(v, &level), S_OK);
        EXPECT(level > 0.24f && level < 0.26f);
        CALL(IAudioEndpointVolume_SetMasterVolumeLevelScalar(v, before, NULL), S_OK);
    }
    IAudioEndpointVolume_Release(v);
    return 0;
}
static int notify_step(void) {
    CALL(IMMDeviceEnumerator_RegisterEndpointNotificationCallback(enumerator, NULL), E_POINTER);
    CALL(IMMDeviceEnumerator_RegisterEndpointNotificationCallback(enumerator, &notify), S_OK);
    CALL(IMMDeviceEnumerator_UnregisterEndpointNotificationCallback(enumerator, &notify), S_OK);
    last = IMMDeviceEnumerator_UnregisterEndpointNotificationCallback(enumerator, &notify);
    raw_hr("unregister-twice", last);
    return 0;
}
static int capture_step(void) {
    IMMDevice *mic = NULL;
    IAudioClient *c = NULL;
    IAudioCaptureClient *cap = NULL;
    WAVEFORMATEX *f = NULL;
    wchar_t *id = NULL;
    UINT32 packet = 0, got = 0, total = 0;
    BYTE *data;
    DWORD flags;
    last = IMMDeviceEnumerator_GetDefaultAudioEndpoint(enumerator, eCapture, eConsole, &mic);
    raw_hr("capture-default", last);
    if (!guest) {
        if (mic) {
            CALL(IMMDevice_GetId(mic, &id), S_OK);
            EXPECT(prefix(id, L"{0.0.1.00000000}.{"));
            CoTaskMemFree(id);
            IMMDevice_Release(mic);
        }
        return 0;
    }
    EXPECT(last == S_OK);
    CALL(IMMDevice_GetId(mic, &id), S_OK);
    EXPECT(prefix(id, L"{0.0.1.00000000}.{"));
    CoTaskMemFree(id);
    /* The microphone's level is the system's (Settings > Sound > Input). */
    {
        IAudioEndpointVolume *v = NULL;
        float level = 0;
        CALL(IMMDevice_Activate(mic, &iid_volume, CLSCTX_ALL, NULL, (void **)&v), S_OK);
        CALL(IAudioEndpointVolume_SetMasterVolumeLevelScalar(v, 0.6f, NULL), S_OK);
        CALL(IAudioEndpointVolume_GetMasterVolumeLevelScalar(v, &level), S_OK);
        EXPECT(level > 0.59f && level < 0.61f);
        Sleep(500);
        IAudioEndpointVolume_Release(v);
    }
    CALL(IMMDevice_Activate(mic, &iid_client, CLSCTX_ALL, NULL, (void **)&c), S_OK);
    CALL(IAudioClient_GetMixFormat(c, &f), S_OK);
    CALL(IAudioClient_Initialize(c, AUDCLNT_SHAREMODE_SHARED, 0, 2000000, 0, f, NULL), S_OK);
    CALL(IAudioClient_GetService(c, &iid_render, (void **)&cap), AUDCLNT_E_WRONG_ENDPOINT_TYPE);
    CALL(IAudioClient_GetService(c, &iid_capture, (void **)&cap), S_OK);
    CALL(IAudioCaptureClient_ReleaseBuffer(cap, 1), AUDCLNT_E_OUT_OF_ORDER);
    CALL(IAudioClient_Start(c), S_OK);
    for (int i = 0; i < 40 && total < f->nSamplesPerSec / 10; i++) {
        Sleep(10);
        CALL(IAudioCaptureClient_GetNextPacketSize(cap, &packet), S_OK);
        while (packet) {
            CALL(IAudioCaptureClient_GetBuffer(cap, &data, &got, &flags, NULL, NULL), S_OK);
            total += got;
            CALL(IAudioCaptureClient_ReleaseBuffer(cap, got), S_OK);
            CALL(IAudioCaptureClient_GetNextPacketSize(cap, &packet), S_OK);
        }
    }
    raw("captured-frames", total);
    EXPECT(total >= f->nSamplesPerSec / 10);
    CALL(IAudioClient_Stop(c), S_OK);
    /* What was captured before Stop stays readable; then the buffer is empty. */
    CALL(IAudioCaptureClient_GetNextPacketSize(cap, &packet), S_OK);
    while (packet) {
        CALL(IAudioCaptureClient_GetBuffer(cap, &data, &got, &flags, NULL, NULL), S_OK);
        CALL(IAudioCaptureClient_ReleaseBuffer(cap, got), S_OK);
        CALL(IAudioCaptureClient_GetNextPacketSize(cap, &packet), S_OK);
    }
    CALL(IAudioCaptureClient_GetBuffer(cap, &data, &got, &flags, NULL, NULL), AUDCLNT_S_BUFFER_EMPTY);
    IAudioCaptureClient_Release(cap);
    CoTaskMemFree(f);
    IAudioClient_Release(c);
    IMMDevice_Release(mic);
    return 0;
}
/* MCI recording (guest only: it records from the microphone): 500 ms of
   48 kHz 16-bit mono into a new WaveAudio element, saved as a WAV file. */
static int mci_step(void) {
    wchar_t reply[64];
    HANDLE file;
    DWORD size;
    if (!guest) return 0;
    CALL(mciSendStringW(L"open new type waveaudio alias rec", reply, 64, NULL), 0);
    CALL(mciSendStringW(L"capability rec can record", reply, 64, NULL), 0);
    EXPECT(wide_equal(reply, L"true"));
    CALL(mciSendStringW(L"set rec bitspersample 16 samplespersec 48000 channels 1", reply, 64, NULL), 0);
    CALL(mciSendStringW(L"record rec to 500 wait", reply, 64, NULL), 0);
    CALL(mciSendStringW(L"status rec length", reply, 64, NULL), 0);
    EXPECT(wide_equal(reply, L"500"));
    CALL(mciSendStringW(L"status rec mode", reply, 64, NULL), 0);
    EXPECT(wide_equal(reply, L"stopped"));
    CALL(mciSendStringW(L"save rec mcirec.wav", reply, 64, NULL), 0);
    CALL(mciSendStringW(L"close rec", reply, 64, NULL), 0);
    file = CreateFileW(L"mcirec.wav", GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    EXPECT(file != INVALID_HANDLE_VALUE);
    size = GetFileSize(file, NULL);
    CloseHandle(file);
    DeleteFileW(L"mcirec.wav");
    raw("mci-file-bytes", size);
    EXPECT(size == 44 + 48000);
    return 0;
}

typedef int (*Step)(void);
static const struct {
    const char *name;
    Step run;
} steps[] = {
    {"enumerator", enumerator_step}, {"default", default_step}, {"collection", collection_step},
    {"properties", properties_step}, {"mix", mix_step},         {"formats", format_step},
    {"initialize", initialize_step}, {"render", render_step},   {"clock", clock_step},
    {"run", run_step},               {"event", event_step},     {"session", session_step},
    {"endpoint", endpoint_step},     {"notify", notify_step},   {"capture", capture_step},
    {"mci-record", mci_step},
};
static int argument_is(const wchar_t *line, const wchar_t *word) {
    /* The last word of the command line. */
    const wchar_t *last_word = line;
    for (const wchar_t *p = line; *p; p++)
        if (*p == L' ' && p[1] && p[1] != L' ') last_word = p + 1;
    return wide_equal(last_word, word);
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    guest = argument_is(line, L"guest");
    if (CoInitializeEx(NULL, COINIT_MULTITHREADED) != S_OK) {
        put("WASAPI-A FAILED CoInitializeEx\r\n");
        ExitProcess(1);
    }
    for (int i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        check = 0;
        step(i + 1, steps[i].name, steps[i].run());
        if (failed_step && guest) break;
    }
    put(failed_step ? "WASAPI-A FAILED\r\n" : "WASAPI-A PASSED\r\n");
    CoUninitialize();
    ExitProcess(failed_step ? 1 : 0);
}

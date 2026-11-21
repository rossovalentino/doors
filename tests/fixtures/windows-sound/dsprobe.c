/* DSOUND-A probe (P15): an ordinary Windows console program (Windows SDK,
 * no C runtime) that plays through dsound.dll:
 *
 *   dsprobe.exe [silent | capture]
 *
 * It enumerates and opens the default device, sets the primary format,
 * then plays four things the acceptance run finds in the guest's captured
 * output: a one-shot 740 Hz buffer (1.5 s) with position notifications, a
 * looping buffer streamed with 990 Hz for two seconds by refilling the half
 * the play cursor has left, two buffers at once (370 Hz at -6 dB panned
 * left and 1480 Hz) for one second, and a 600 Hz buffer recorded at 24 kHz
 * played with SetFrequency(48000), so it sounds at 1200 Hz. "silent" writes
 * zero samples, so the same binary runs on Windows as reference without
 * making a sound.
 *
 * One "DSOUND-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <mmsystem.h>
#include <dsound.h>
#include <intrin.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
    return destination;
}

static const GUID iid_buffer8 = {0x6825a449, 0x7524, 0x4d82, {0x92, 0x0f, 0x50, 0xe3, 0x6a, 0xb3, 0xab, 0x1e}};
static const GUID iid_notify = {0xb0210783, 0x89cd, 0x11d0, {0xaf, 0x08, 0x00, 0xa0, 0xc9, 0x25, 0xcd, 0x16}};
static const GUID default_playback = {0xdef00000, 0x9c6d, 0x47ed, {0xaa, 0xf1, 0x4d, 0xda, 0x8f, 0x2b, 0x5c, 0x03}};

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
static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static int same_guid(const GUID *a, const GUID *b) {
    const unsigned char *x = (const unsigned char *)a, *y = (const unsigned char *)b;
    for (int i = 0; i < 16; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}
static int failed_step;
/* The tag of every result line (DSOUND-A, or DSCAP-A in capture mode). */
static const char *tag = "DSOUND-A";
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

static int silent;
/* A triangle tone at `hz` into `frames` frames of `channels` 16-bit
 * channels, amplitude 8000, continuing from *phase (integer only). */
static void tone(short *samples, DWORD frames, WORD channels, DWORD rate, DWORD hz, unsigned int *phase) {
    unsigned int increment = (unsigned int)(((unsigned __int64)hz << 32) / rate);
    for (DWORD i = 0; i < frames; i++) {
        int p = (int)(*phase >> 16);
        int v = p < 32768 ? p - 16384 : 49151 - p;
        short s = silent ? 0 : (short)(v * 8000 / 16384);
        for (WORD c = 0; c < channels; c++) samples[i * channels + c] = s;
        *phase += increment;
    }
}
static void format(WAVEFORMATEX *f, DWORD rate, WORD channels) {
    memset(f, 0, sizeof *f);
    f->wFormatTag = WAVE_FORMAT_PCM;
    f->nChannels = channels;
    f->nSamplesPerSec = rate;
    f->wBitsPerSample = 16;
    f->nBlockAlign = (WORD)(channels * 2);
    f->nAvgBytesPerSec = rate * f->nBlockAlign;
}
static void describe(DSBUFFERDESC *d, DWORD flags, DWORD bytes, WAVEFORMATEX *f) {
    memset(d, 0, sizeof *d);
    d->dwSize = sizeof *d;
    d->dwFlags = flags;
    d->dwBufferBytes = bytes;
    d->lpwfxFormat = f;
}
/* Writes `frames` of a tone at the start of `buffer` through Lock. */
static HRESULT fill(IDirectSoundBuffer *buffer, DWORD offset, DWORD bytes, WORD channels, DWORD rate, DWORD hz, unsigned int *phase) {
    void *a, *b;
    DWORD na, nb;
    HRESULT h = IDirectSoundBuffer_Lock(buffer, offset, bytes, &a, &na, &b, &nb, 0);
    if (FAILED(h)) return h;
    tone((short *)a, na / (channels * 2), channels, rate, hz, phase);
    if (b) tone((short *)b, nb / (channels * 2), channels, rate, hz, phase);
    return IDirectSoundBuffer_Unlock(buffer, a, na, b, nb);
}
/* Creates a secondary buffer holding `seconds_tenths` of a tone. */
static IDirectSoundBuffer *tone_buffer(IDirectSound8 *ds, DWORD flags, WORD channels, DWORD rate, DWORD hz, DWORD tenths, HRESULT *h) {
    WAVEFORMATEX f;
    DSBUFFERDESC d;
    IDirectSoundBuffer *buffer = NULL;
    unsigned int phase = 0;
    format(&f, rate, channels);
    describe(&d, flags, rate / 10 * tenths * f.nBlockAlign, &f);
    *h = IDirectSound8_CreateSoundBuffer(ds, &d, &buffer, NULL);
    if (SUCCEEDED(*h)) *h = fill(buffer, 0, d.dwBufferBytes, channels, rate, hz, &phase);
    return buffer;
}

static int devices, named;
static BOOL CALLBACK enumerate(LPGUID guid, LPCWSTR description, LPCWSTR module, LPVOID context) {
    (void)module;
    (void)context;
    devices++;
    if (guid && description && description[0]) named++;
    return TRUE;
}


/* dsprobe.exe capture (DSCAP-A): records through DirectSoundCapture from
 * the guest's silent input; never run on the owner's PC. */
static void capture_mode(void) {
    tag = "DSCAP-A";
    /* 1: the primary capture driver and the microphone. */
    devices = named = 0;
    HRESULT e = DirectSoundCaptureEnumerateW(enumerate, NULL);
    step(1, "cap-enumerate", e == DS_OK && devices >= 2 && named >= 1, (long)devices * 100 + named);

    /* 2: the default capture device and its capabilities. */
    IDirectSoundCapture *dsc = NULL;
    HRESULT c = DirectSoundCaptureCreate8(NULL, &dsc, NULL);
    DSCCAPS caps;
    memset(&caps, 0, sizeof caps);
    caps.dwSize = sizeof caps;
    HRESULT k = dsc ? IDirectSoundCapture_GetCaps(dsc, &caps) : c;
    step(2, "cap-create", c == DS_OK && k == DS_OK && caps.dwChannels >= 1 && caps.dwFormats != 0, (long)c);
    if (!dsc) ExitProcess(2);

    /* 3: one second of 44.1 kHz 16-bit mono, notified at its middle and
     *    at the stop; the format reads back. */
    WAVEFORMATEX f, got;
    format(&f, 44100, 1);
    DSCBUFFERDESC d;
    memset(&d, 0, sizeof d);
    d.dwSize = sizeof d;
    d.dwBufferBytes = 44100 * 2;
    d.lpwfxFormat = &f;
    IDirectSoundCaptureBuffer *buffer = NULL;
    IDirectSoundNotify *notify = NULL;
    HRESULT b = IDirectSoundCapture_CreateCaptureBuffer(dsc, &d, &buffer, NULL);
    HANDLE middle = CreateEventW(NULL, FALSE, FALSE, NULL);
    HANDLE stopped = CreateEventW(NULL, FALSE, FALSE, NULL);
    DSBPOSITIONNOTIFY marks[2] = {{44100, middle}, {DSBPN_OFFSETSTOP, stopped}};
    if (SUCCEEDED(b)) b = IDirectSoundCaptureBuffer_QueryInterface(buffer, &iid_notify, (void **)&notify);
    if (SUCCEEDED(b)) b = IDirectSoundNotify_SetNotificationPositions(notify, 2, marks);
    DWORD written = 0;
    memset(&got, 0, sizeof got);
    if (SUCCEEDED(b)) b = IDirectSoundCaptureBuffer_GetFormat(buffer, &got, sizeof got, &written);
    step(3, "cap-buffer", b == DS_OK && got.nSamplesPerSec == 44100 && got.nChannels == 1 && written == sizeof got, (long)b);
    if (!buffer) ExitProcess(3);

    /* 4: a one-shot capture fills the buffer once and stops by itself. */
    DWORD start = GetTickCount();
    HRESULT s = IDirectSoundCaptureBuffer_Start(buffer, 0);
    Sleep(300);
    DWORD status = 0, cursor = 0, readable = 0;
    IDirectSoundCaptureBuffer_GetStatus(buffer, &status);
    IDirectSoundCaptureBuffer_GetCurrentPosition(buffer, &cursor, &readable);
    int capturing = (status & DSCBSTATUS_CAPTURING) != 0 && cursor > 0 && readable <= 44100 * 2;
    DWORD half = WaitForSingleObject(middle, 3000);
    DWORD end = WaitForSingleObject(stopped, 3000);
    DWORD elapsed = GetTickCount() - start;
    IDirectSoundCaptureBuffer_GetStatus(buffer, &status);
    step(4, "capture", s == DS_OK && capturing && half == WAIT_OBJECT_0 && end == WAIT_OBJECT_0 && elapsed >= 900 && elapsed < 3000 && status == 0,
         (long)elapsed);

    /* 5: looping capture keeps going past the end until Stop. */
    s = IDirectSoundCaptureBuffer_Start(buffer, DSCBSTART_LOOPING);
    Sleep(1500);
    IDirectSoundCaptureBuffer_GetStatus(buffer, &status);
    IDirectSoundCaptureBuffer_GetCurrentPosition(buffer, &cursor, NULL);
    int looping = status == (DSCBSTATUS_CAPTURING | DSCBSTATUS_LOOPING) && cursor < 44100 * 2;
    HRESULT t = IDirectSoundCaptureBuffer_Stop(buffer);
    DWORD stop_event = WaitForSingleObject(stopped, 500);
    IDirectSoundCaptureBuffer_GetStatus(buffer, &status);
    void *p1, *p2;
    DWORD n1, n2;
    HRESULT l = IDirectSoundCaptureBuffer_Lock(buffer, 0, 0, &p1, &n1, &p2, &n2, DSCBLOCK_ENTIREBUFFER);
    if (SUCCEEDED(l)) IDirectSoundCaptureBuffer_Unlock(buffer, p1, n1, p2, n2);
    step(5, "cap-looping", s == DS_OK && looping && t == DS_OK && stop_event == WAIT_OBJECT_0 && status == 0 && l == DS_OK && n1 == 44100 * 2,
         (long)cursor);

    /* 6: Release of everything ends at zero. */
    IDirectSoundNotify_Release(notify);
    ULONG left = IDirectSoundCaptureBuffer_Release(buffer);
    ULONG last = IDirectSoundCapture_Release(dsc);
    step(6, "cap-release", left == 0 && last == 0, (long)last);
    put(failed_step ? "DSCAP-A FAILED\r\n" : "DSCAP-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}
void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    const wchar_t *line = GetCommandLineW();
    for (const wchar_t *p = line; *p; p++)
        if (p[0] == L's' && wide_equal(p, L"silent")) silent = 1;
    for (const wchar_t *p = line; *p; p++)
        if (p[0] == L'c' && wide_equal(p, L"capture")) capture_mode();

    /* 1: the primary driver (no GUID) and at least one named device. */
    HRESULT e = DirectSoundEnumerateW(enumerate, NULL);
    step(1, "enumerate", e == DS_OK && devices >= 2 && named >= 1, (long)devices * 100 + named);

    /* 2: GetDeviceID maps the default-playback GUID to a real device. */
    GUID device;
    HRESULT g = GetDeviceID(&default_playback, &device);
    step(2, "device-id", g == DS_OK && !same_guid(&device, &default_playback), (long)g);

    /* 3: the default device opens; no cooperative level yet refuses buffers. */
    IDirectSound8 *ds = NULL;
    HRESULT c = DirectSoundCreate8(NULL, &ds, NULL);
    if (FAILED(c) || !ds) {
        step(3, "create", 0, (long)c);
        ExitProcess(3);
    }
    /* A hidden window of its own for the cooperative level (a console
     * program has none). */
    HWND window = CreateWindowExW(0, L"Static", L"dsprobe", WS_POPUP, 0, 0, 1, 1, NULL, NULL, NULL, NULL);
    HRESULT p = window ? IDirectSound8_SetCooperativeLevel(ds, window, DSSCL_PRIORITY) : E_FAIL;
    step(3, "create", p == DS_OK, (long)p);

    /* 4: capabilities: 16-bit stereo secondary buffers, a rate range. */
    DSCAPS caps;
    memset(&caps, 0, sizeof caps);
    caps.dwSize = sizeof caps;
    HRESULT k = IDirectSound8_GetCaps(ds, &caps);
    step(4, "caps",
         k == DS_OK && (caps.dwFlags & DSCAPS_SECONDARY16BIT) && (caps.dwFlags & DSCAPS_SECONDARYSTEREO) &&
             caps.dwMinSecondarySampleRate <= 8000 && caps.dwMaxSecondarySampleRate >= 96000,
         (long)caps.dwFlags);

    /* 5: the primary buffer takes 48 kHz 16-bit stereo and reports it. */
    DSBUFFERDESC d;
    IDirectSoundBuffer *primary = NULL;
    describe(&d, DSBCAPS_PRIMARYBUFFER, 0, NULL);
    HRESULT pc = IDirectSound8_CreateSoundBuffer(ds, &d, &primary, NULL);
    WAVEFORMATEX want, got;
    format(&want, 48000, 2);
    HRESULT sf = SUCCEEDED(pc) ? IDirectSoundBuffer_SetFormat(primary, &want) : pc;
    DWORD written = 0;
    memset(&got, 0, sizeof got);
    HRESULT gf = SUCCEEDED(sf) ? IDirectSoundBuffer_GetFormat(primary, &got, sizeof got, &written) : sf;
    step(5, "primary", gf == DS_OK && got.nSamplesPerSec == 48000 && got.nChannels == 2 && got.wBitsPerSample == 16,
         (long)got.nSamplesPerSec);

    /* 6: refused buffers: three channels without WAVEFORMATEXTENSIBLE and
     *    an empty buffer with DSERR_INVALIDPARAM, a compressed (ADPCM) format
     *    with DSERR_UNSUPPORTED (as Windows answers). */
    WAVEFORMATEX bad;
    IDirectSoundBuffer *none = NULL;
    format(&bad, 44100, 3);
    describe(&d, 0, 4410 * 6, &bad);
    HRESULT b1 = IDirectSound8_CreateSoundBuffer(ds, &d, &none, NULL);
    format(&bad, 44100, 2);
    describe(&d, 0, 0, &bad);
    HRESULT b2 = IDirectSound8_CreateSoundBuffer(ds, &d, &none, NULL);
    format(&bad, 44100, 1);
    bad.wFormatTag = 2;
    bad.wBitsPerSample = 4;
    bad.nBlockAlign = 256;
    bad.nAvgBytesPerSec = 22311;
    describe(&d, 0, 256 * 16, &bad);
    HRESULT b3 = IDirectSound8_CreateSoundBuffer(ds, &d, &none, NULL);
    step(6, "bad-buffers", b1 == DSERR_INVALIDPARAM && b2 == DSERR_INVALIDPARAM && b3 == DSERR_UNSUPPORTED, (long)b3);

    /* 7: one shot, 1.5 s of 740 Hz at 44.1 kHz mono: notifications at the
     *    middle and at the stop, then not playing. */
    HRESULT h;
    IDirectSoundBuffer *shot = tone_buffer(ds, DSBCAPS_CTRLPOSITIONNOTIFY | DSBCAPS_GETCURRENTPOSITION2, 1, 44100, 740, 15, &h);
    IDirectSoundNotify *notify = NULL;
    HANDLE middle = CreateEventW(NULL, FALSE, FALSE, NULL);
    HANDLE stopped = CreateEventW(NULL, FALSE, FALSE, NULL);
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_QueryInterface(shot, &iid_notify, (void **)&notify);
    DSBPOSITIONNOTIFY marks[2] = {{44100 * 2 * 15 / 20, middle}, {DSBPN_OFFSETSTOP, stopped}};
    if (SUCCEEDED(h)) h = IDirectSoundNotify_SetNotificationPositions(notify, 2, marks);
    DWORD start = GetTickCount();
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_Play(shot, 0, 0, 0);
    DWORD status = 0;
    IDirectSoundBuffer_GetStatus(shot, &status);
    int playing = (status & DSBSTATUS_PLAYING) != 0;
    DWORD half = WaitForSingleObject(middle, 3000);
    DWORD half_at = GetTickCount() - start;
    DWORD end = WaitForSingleObject(stopped, 3000);
    DWORD end_at = GetTickCount() - start;
    IDirectSoundBuffer_GetStatus(shot, &status);
    step(7, "one-shot",
         h == DS_OK && playing && half == WAIT_OBJECT_0 && end == WAIT_OBJECT_0 && half_at >= 500 && half_at < end_at && end_at >= 1300 &&
             end_at < 3000 && !(status & DSBSTATUS_PLAYING),
         (long)half_at * 10000 + (long)end_at);

    /* 8: streaming: a looping half-second buffer at 22.05 kHz stereo, its
     *    halves refilled with 990 Hz as the play cursor leaves them, for two
     *    seconds (four refills); the cursors move and stay in the buffer. */
    WAVEFORMATEX sf2;
    format(&sf2, 22050, 2);
    DWORD size = 22050 / 2 * 4;
    describe(&d, DSBCAPS_CTRLPOSITIONNOTIFY | DSBCAPS_GETCURRENTPOSITION2, size, &sf2);
    IDirectSoundBuffer *stream = NULL;
    IDirectSoundNotify *stream_notify = NULL;
    HANDLE halves[2] = {CreateEventW(NULL, FALSE, FALSE, NULL), CreateEventW(NULL, FALSE, FALSE, NULL)};
    unsigned int phase = 0;
    h = IDirectSound8_CreateSoundBuffer(ds, &d, &stream, NULL);
    if (SUCCEEDED(h)) h = fill(stream, 0, size, 2, 22050, 990, &phase);
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_QueryInterface(stream, &iid_notify, (void **)&stream_notify);
    DSBPOSITIONNOTIFY at[2] = {{0, halves[1]}, {size / 2, halves[0]}};
    if (SUCCEEDED(h)) h = IDirectSoundNotify_SetNotificationPositions(stream_notify, 2, at);
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_Play(stream, 0, 0, DSBPLAY_LOOPING);
    int refills = 0, cursors_ok = 1;
    DWORD last_play = 0;
    int moved = 0;
    start = GetTickCount();
    while (SUCCEEDED(h) && GetTickCount() - start < 2000) {
        DWORD which = WaitForMultipleObjects(2, halves, FALSE, 1000);
        if (which > WAIT_OBJECT_0 + 1) {
            h = E_FAIL;
            break;
        }
        /* Event 0: the cursor passed the middle, so refill the first half. */
        h = fill(stream, which == WAIT_OBJECT_0 ? 0 : size / 2, size / 2, 2, 22050, 990, &phase);
        refills++;
        DWORD play_at = 0, write_at = 0;
        IDirectSoundBuffer_GetCurrentPosition(stream, &play_at, &write_at);
        if (play_at >= size || write_at >= size) cursors_ok = 0;
        if (play_at != last_play) moved++;
        last_play = play_at;
    }
    IDirectSoundBuffer_GetStatus(stream, &status);
    int looping = (status & DSBSTATUS_LOOPING) != 0;
    IDirectSoundBuffer_Stop(stream);
    step(8, "streaming", h == DS_OK && refills >= 3 && cursors_ok && moved >= 2 && looping, (long)refills * 100 + moved);

    /* 9: two buffers at once for one second: 370 Hz at -6 dB panned left
     *    and 1480 Hz; volume and pan read back. */
    HRESULT h1, h2;
    IDirectSoundBuffer *low = tone_buffer(ds, DSBCAPS_CTRLVOLUME | DSBCAPS_CTRLPAN, 1, 44100, 370, 10, &h1);
    IDirectSoundBuffer *high = tone_buffer(ds, DSBCAPS_CTRLVOLUME, 1, 44100, 1480, 10, &h2);
    LONG volume = 0, pan = 0, refused = 0;
    if (SUCCEEDED(h1)) h1 = IDirectSoundBuffer_SetVolume(low, -600);
    if (SUCCEEDED(h1)) h1 = IDirectSoundBuffer_SetPan(low, -2000);
    if (SUCCEEDED(h1)) IDirectSoundBuffer_GetVolume(low, &volume);
    if (SUCCEEDED(h1)) IDirectSoundBuffer_GetPan(low, &pan);
    /* `high` has no pan control. */
    refused = SUCCEEDED(h2) && IDirectSoundBuffer_SetPan(high, 100) == DSERR_CONTROLUNAVAIL;
    if (SUCCEEDED(h1)) h1 = IDirectSoundBuffer_Play(low, 0, 0, 0);
    if (SUCCEEDED(h2)) h2 = IDirectSoundBuffer_Play(high, 0, 0, 0);
    Sleep(1500);
    DWORD s1 = 0, s2 = 0;
    IDirectSoundBuffer_GetStatus(low, &s1);
    IDirectSoundBuffer_GetStatus(high, &s2);
    step(9, "mixed",
         h1 == DS_OK && h2 == DS_OK && volume == -600 && pan == -2000 && refused && !(s1 & DSBSTATUS_PLAYING) && !(s2 & DSBSTATUS_PLAYING),
         (long)volume);

    /* 10: a 600 Hz tone recorded at 24 kHz, played at 48 kHz: 1200 Hz. */
    IDirectSoundBuffer *fast = tone_buffer(ds, DSBCAPS_CTRLFREQUENCY, 1, 24000, 600, 20, &h);
    DWORD frequency = 0;
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_SetFrequency(fast, 48000);
    if (SUCCEEDED(h)) IDirectSoundBuffer_GetFrequency(fast, &frequency);
    HRESULT range = SUCCEEDED(h) ? IDirectSoundBuffer_SetFrequency(fast, 50) : h;
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_Play(fast, 0, 0, 0);
    /* One second of sound: it stops by itself within 2.5 s. */
    start = GetTickCount();
    DWORD took = 0;
    do {
        Sleep(20);
        IDirectSoundBuffer_GetStatus(fast, &status);
        took = GetTickCount() - start;
    } while ((status & DSBSTATUS_PLAYING) && took < 2500);
    step(10, "frequency", h == DS_OK && frequency == 48000 && range == DSERR_INVALIDPARAM && !(status & DSBSTATUS_PLAYING) && took >= 900,
         (long)took);

    /* 11: Stop holds the cursor; Play resumes; SetCurrentPosition moves it. */
    IDirectSoundBuffer *held = tone_buffer(ds, 0, 1, 44100, 740, 20, &h);
    if (SUCCEEDED(h)) h = IDirectSoundBuffer_SetVolume(held, -100);
    int no_control = h == DSERR_CONTROLUNAVAIL;
    h = held ? IDirectSoundBuffer_Play(held, 0, 0, 0) : E_FAIL;
    Sleep(300);
    IDirectSoundBuffer_Stop(held);
    DWORD stop_play = 0, later_play = 0, moved_play = 0;
    IDirectSoundBuffer_GetCurrentPosition(held, &stop_play, NULL);
    Sleep(200);
    IDirectSoundBuffer_GetCurrentPosition(held, &later_play, NULL);
    HRESULT sp = IDirectSoundBuffer_SetCurrentPosition(held, 1000);
    IDirectSoundBuffer_GetCurrentPosition(held, &moved_play, NULL);
    step(11, "stop-position",
         h == DS_OK && no_control && stop_play > 0 && stop_play == later_play && sp == DS_OK && moved_play == 1000,
         (long)stop_play);

    /* 12: interfaces: IDirectSoundBuffer8 and back; Release of everything
     *     ends at zero. */
    IDirectSoundBuffer8 *eight = NULL;
    HRESULT q = IDirectSoundBuffer_QueryInterface(held, &iid_buffer8, (void **)&eight);
    ULONG after = eight ? IDirectSoundBuffer8_Release(eight) : 99;
    IDirectSoundNotify_Release(notify);
    IDirectSoundNotify_Release(stream_notify);
    IDirectSoundBuffer_Release(shot);
    IDirectSoundBuffer_Release(stream);
    IDirectSoundBuffer_Release(low);
    IDirectSoundBuffer_Release(high);
    IDirectSoundBuffer_Release(fast);
    IDirectSoundBuffer_Release(held);
    IDirectSoundBuffer_Release(primary);
    ULONG last = IDirectSound8_Release(ds);
    step(12, "release", q == DS_OK && after == 1 && last == 0, (long)last);

    DestroyWindow(window);
    put(failed_step ? "DSOUND-A FAILED\r\n" : "DSOUND-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

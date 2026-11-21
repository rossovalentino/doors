/* SOUND-A probe (P15): an ordinary Windows console program (Windows SDK,
 * no C runtime) that plays through winmm.dll:
 *
 *   soundprobe.exe [silent]
 *
 * waveOut plays two seconds of a 660 Hz triangle tone in four buffers and
 * PlaySound one second of 880 Hz from a WAV image in memory; the acceptance
 * run finds both in the guest's captured output. "silent" writes zero
 * samples and skips the system sounds, so the same binary runs on Windows
 * as reference without making a sound or changing a setting.
 *
 *   soundprobe.exe capture   records through waveIn (MIC-A; the guest's input
 *                            is silent); never run on the owner's PC
 *   soundprobe.exe denied    opens waveIn while the microphone policy
 *                            refuses desktop apps (MIC-D)
 *
 * One "SOUND-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <mmsystem.h>
#include <intrin.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
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
static int failed_step;
/* The tag of every result line (SOUND-A, or MIC-A / MIC-D in their modes). */
static const char *tag = "SOUND-A";
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
/* A triangle tone at `hz`, amplitude 8000, by a 32-bit phase accumulator
 * (integer only: no floating point without a C runtime). */
static void tone(short *samples, DWORD count, DWORD rate, DWORD hz) {
    unsigned int phase = 0;
    unsigned int increment = (unsigned int)(((unsigned __int64)hz << 32) / rate);
    for (DWORD i = 0; i < count; i++) {
        int p = (int)(phase >> 16);
        int v = p < 32768 ? p - 16384 : 49151 - p;
        samples[i] = silent ? 0 : (short)(v * 8000 / 16384);
        phase += increment;
    }
}
static void *allocate(SIZE_T bytes) {
    return VirtualAlloc(NULL, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
}

static volatile LONG opened, done, closed;
static void CALLBACK wave_proc(HWAVEOUT device, UINT message, DWORD_PTR instance, DWORD_PTR a, DWORD_PTR b) {
    (void)device;
    (void)instance;
    (void)a;
    (void)b;
    if (message == WOM_OPEN) InterlockedIncrement(&opened);
    if (message == WOM_DONE) InterlockedIncrement(&done);
    if (message == WOM_CLOSE) InterlockedIncrement(&closed);
}
static int wait_count(volatile LONG *counter, LONG wanted, DWORD millis) {
    DWORD start = GetTickCount();
    while (*counter < wanted) {
        if (GetTickCount() - start > millis) return 0;
        Sleep(5);
    }
    return 1;
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
static int all_done(WAVEHDR *headers, int count) {
    for (int i = 0; i < count; i++)
        if (!(headers[i].dwFlags & WHDR_DONE)) return 0;
    return 1;
}
static volatile LONG ticks;
static void CALLBACK timer_proc(UINT id, UINT message, DWORD_PTR user, DWORD_PTR a, DWORD_PTR b) {
    (void)id;
    (void)message;
    (void)user;
    (void)a;
    (void)b;
    InterlockedIncrement(&ticks);
}

/* ---- capture (MIC-A) and denial (MIC-D) modes -------------------------- */
static volatile LONG in_opened, in_data, in_closed;
static void CALLBACK wave_in_proc(HWAVEIN device, UINT message, DWORD_PTR instance, DWORD_PTR a, DWORD_PTR b) {
    (void)device;
    (void)instance;
    (void)a;
    (void)b;
    if (message == WIM_OPEN) InterlockedIncrement(&in_opened);
    if (message == WIM_DATA) InterlockedIncrement(&in_data);
    if (message == WIM_CLOSE) InterlockedIncrement(&in_closed);
}
/* The guest's input (QEMU's none backend) is silence: the largest sample. */
static long loudest(const short *samples, DWORD count) {
    long peak = 0;
    for (DWORD i = 0; i < count; i++) {
        long v = samples[i] < 0 ? -(long)samples[i] : samples[i];
        if (v > peak) peak = v;
    }
    return peak;
}
static void capture_mode(void) {
    tag = "MIC-A";
    /* 1: one input device, its mapper's and its own capabilities. */
    WAVEINCAPSW mapper, mic;
    UINT inputs = waveInGetNumDevs();
    MMRESULT c1 = waveInGetDevCapsW(WAVE_MAPPER, &mapper, sizeof mapper);
    MMRESULT c2 = waveInGetDevCapsW(0, &mic, sizeof mic);
    step(1, "in-devices",
         inputs == 1 && c1 == 0 && c2 == 0 && mapper.wMid == 1 && mapper.wPid == 2 && mapper.vDriverVersion == 0x500 &&
             mapper.dwFormats == 0xFFFFF && mapper.wChannels == 2 && mic.wPid == 65535 && mic.szPname[0] &&
             sizeof mapper == 80 && waveInGetDevCapsW(inputs + 3, &mic, sizeof mic) == MMSYSERR_BADDEVICEID,
         (long)inputs);

    /* 2: open 44.1 kHz mono 16-bit with a callback function: WIM_OPEN. */
    WAVEFORMATEX f;
    format(&f, 44100, 1);
    HWAVEIN device = NULL;
    MMRESULT o = waveInOpen(&device, WAVE_MAPPER, &f, (DWORD_PTR)wave_in_proc, 0, CALLBACK_FUNCTION);
    step(2, "in-open", o == 0 && device && wait_count(&in_opened, 1, 1000), (long)o);

    /* 3: a second recorded into four quarter-second buffers, in real time,
     * every buffer full; the input is silent. */
    enum { QUARTER = 11025 };
    short *recorded = allocate(4 * QUARTER * sizeof(short));
    WAVEHDR headers[4];
    memset(headers, 0, sizeof headers);
    int queued = recorded != NULL && o == 0;
    for (int i = 0; i < 4 && queued; i++) {
        headers[i].lpData = (LPSTR)(recorded + i * QUARTER);
        headers[i].dwBufferLength = QUARTER * sizeof(short);
        queued = waveInPrepareHeader(device, &headers[i], sizeof headers[i]) == 0 &&
                 waveInAddBuffer(device, &headers[i], sizeof headers[i]) == 0;
    }
    DWORD started = GetTickCount(), elapsed = 0;
    int full = 0;
    if (queued && waveInStart(device) == 0) {
        while (!all_done(headers, 4) && GetTickCount() - started < 4000) Sleep(10);
        elapsed = GetTickCount() - started;
        full = 1;
        for (int i = 0; i < 4; i++)
            if (headers[i].dwBytesRecorded != QUARTER * sizeof(short)) full = 0;
    }
    long peak = recorded ? loudest(recorded, 4 * QUARTER) : -1;
    step(3, "record",
         queued && all_done(headers, 4) && full && wait_count(&in_data, 4, 1000) && elapsed >= 900 && elapsed < 3500 && peak <= 1,
         (long)elapsed);

    /* 4: the position counts what was recorded: 44100 samples. */
    MMTIME samples;
    memset(&samples, 0, sizeof samples);
    samples.wType = TIME_SAMPLES;
    MMRESULT p = waveInGetPosition(device, &samples, sizeof samples);
    step(4, "in-position", p == 0 && samples.wType == TIME_SAMPLES && samples.u.sample >= 44100 && samples.u.sample <= 44100 + 4410,
         (long)samples.u.sample);

    /* 5: waveInStop returns the partly filled buffer and keeps the empty
     * one queued; closing refuses while it is; waveInReset returns it and
     * rewinds. */
    for (int i = 0; i < 4; i++) waveInUnprepareHeader(device, &headers[i], sizeof headers[i]);
    in_data = 0;
    WAVEHDR two[2];
    memset(two, 0, sizeof two);
    for (int i = 0; i < 2 && recorded; i++) {
        two[i].lpData = (LPSTR)recorded;
        two[i].dwBufferLength = 2 * QUARTER * sizeof(short);
        waveInPrepareHeader(device, &two[i], sizeof two[i]);
        waveInAddBuffer(device, &two[i], sizeof two[i]);
    }
    waveInStart(device);
    Sleep(300);
    MMRESULT stop = waveInStop(device);
    int partial = (two[0].dwFlags & WHDR_DONE) && two[0].dwBytesRecorded > 0 && two[0].dwBytesRecorded < two[0].dwBufferLength &&
                  (two[1].dwFlags & WHDR_INQUEUE);
    MMRESULT busy = waveInClose(device);
    MMRESULT reset = waveInReset(device);
    int returned = (two[1].dwFlags & WHDR_DONE) && !(two[1].dwFlags & WHDR_INQUEUE);
    memset(&samples, 0, sizeof samples);
    samples.wType = TIME_SAMPLES;
    waveInGetPosition(device, &samples, sizeof samples);
    for (int i = 0; i < 2; i++) waveInUnprepareHeader(device, &two[i], sizeof two[i]);
    MMRESULT c = waveInClose(device);
    step(5, "stop-reset",
         stop == 0 && partial && busy == WAVERR_STILLPLAYING && reset == 0 && returned && samples.u.sample == 0 && c == 0 &&
             wait_count(&in_data, 2, 1000) && wait_count(&in_closed, 1, 1000),
         (long)two[0].dwBytesRecorded);

    /* 6: a longer recording, while the acceptance run looks at the taskbar
     * and Settings ("Currently in use"). */
    device = NULL;
    MMRESULT o2 = waveInOpen(&device, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    short *hold = allocate(8 * 44100 * sizeof(short));
    WAVEHDR held;
    memset(&held, 0, sizeof held);
    int holding = o2 == 0 && hold != NULL;
    if (holding) {
        held.lpData = (LPSTR)hold;
        held.dwBufferLength = 8 * 44100 * sizeof(short);
        holding = waveInPrepareHeader(device, &held, sizeof held) == 0 && waveInAddBuffer(device, &held, sizeof held) == 0 &&
                  waveInStart(device) == 0;
    }
    put("MIC-A holding\r\n");
    started = GetTickCount();
    while (holding && !(held.dwFlags & WHDR_DONE) && GetTickCount() - started < 12000) Sleep(20);
    DWORD held_for = GetTickCount() - started;
    int complete = (held.dwFlags & WHDR_DONE) && held.dwBytesRecorded == held.dwBufferLength;
    if (device) {
        waveInReset(device);
        waveInUnprepareHeader(device, &held, sizeof held);
        waveInClose(device);
    }
    step(6, "hold", holding && complete, (long)held_for);
    put(failed_step ? "MIC-A FAILED\r\n" : "MIC-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}
static void denied_mode(void) {
    tag = "MIC-D";
    /* 1: with "Let desktop apps access your microphone" off the device is
     * still there, but opening it fails with MMSYSERR_ERROR. */
    WAVEFORMATEX f;
    format(&f, 44100, 1);
    HWAVEIN device = NULL;
    UINT inputs = waveInGetNumDevs();
    MMRESULT o = waveInOpen(&device, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    if (o == 0) waveInClose(device);
    step(1, "denied", inputs == 1 && o == MMSYSERR_ERROR, (long)o);
    put(failed_step ? "MIC-D FAILED\r\n" : "MIC-D PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    const wchar_t *line = GetCommandLineW();
    for (const wchar_t *p = line; *p; p++)
        if (p[0] == L's' && wide_equal(p, L"silent")) silent = 1;
    for (const wchar_t *p = line; *p; p++) {
        if (p[0] == L'c' && wide_equal(p, L"capture")) capture_mode();
        if (p[0] == L'd' && wide_equal(p, L"denied")) denied_mode();
    }

    /* 1: one output device; capture devices are reported, not required. */
    UINT outputs = waveOutGetNumDevs();
    step(1, "devices", outputs >= 1, (long)outputs * 100 + (long)waveInGetNumDevs());

    /* 2: the sound mapper's and the device's capabilities. */
    WAVEOUTCAPSW mapper, speaker;
    WAVEOUTCAPSA narrow;
    MMRESULT r1 = waveOutGetDevCapsW(WAVE_MAPPER, &mapper, sizeof mapper);
    MMRESULT r2 = waveOutGetDevCapsW(0, &speaker, sizeof speaker);
    MMRESULT r3 = waveOutGetDevCapsA(0, &narrow, sizeof narrow);
    MMRESULT r4 = waveOutGetDevCapsW(outputs + 5, &speaker, sizeof speaker);
    step(2, "caps",
         r1 == 0 && r2 == 0 && r3 == 0 && r4 == MMSYSERR_BADDEVICEID && mapper.wMid == 1 && mapper.wPid == 2 &&
             mapper.vDriverVersion == 0x500 && mapper.dwFormats == 0xFFFFF && mapper.wChannels == 2 &&
             (mapper.dwSupport & WAVECAPS_VOLUME) && speaker.wMid == 1 && speaker.wChannels == 2 && speaker.szPname[0] &&
             narrow.szPname[0] && sizeof narrow == 52 && sizeof mapper == 84,
         (long)mapper.dwSupport);

    /* 3: format queries open nothing; an unknown tag is refused. */
    WAVEFORMATEX f;
    format(&f, 44100, 2);
    MMRESULT q1 = waveOutOpen(NULL, WAVE_MAPPER, &f, 0, 0, WAVE_FORMAT_QUERY);
    f.wFormatTag = 0x1234;
    MMRESULT q2 = waveOutOpen(NULL, WAVE_MAPPER, &f, 0, 0, WAVE_FORMAT_QUERY);
    step(3, "format-query", q1 == 0 && q2 == WAVERR_BADFORMAT, (long)q1 * 100 + (long)q2);

    /* 4: open 44.1 kHz mono with a callback function: WOM_OPEN arrives. */
    HWAVEOUT device = NULL;
    format(&f, 44100, 1);
    MMRESULT o = waveOutOpen(&device, WAVE_MAPPER, &f, (DWORD_PTR)wave_proc, 0, CALLBACK_FUNCTION);
    step(4, "open-callback", o == 0 && device && wait_count(&opened, 1, 1000), (long)o);

    /* 5: four half-second buffers of 660 Hz play in real time. */
    enum { HALF = 22050 };
    short *tone660 = allocate(4 * HALF * sizeof(short));
    WAVEHDR headers[4];
    DWORD started = GetTickCount();
    int written = tone660 != NULL && o == 0;
    if (written) {
        tone(tone660, 4 * HALF, 44100, 660);
        for (int i = 0; i < 4; i++) {
            memset(&headers[i], 0, sizeof headers[i]);
            headers[i].lpData = (LPSTR)(tone660 + i * HALF);
            headers[i].dwBufferLength = HALF * sizeof(short);
            if (waveOutPrepareHeader(device, &headers[i], sizeof headers[i]) != 0 || !(headers[i].dwFlags & WHDR_PREPARED))
                written = 0;
            else if (waveOutWrite(device, &headers[i], sizeof headers[i]) != 0)
                written = 0;
        }
    }
    DWORD elapsed = 0;
    if (written) {
        while (!all_done(headers, 4) && GetTickCount() - started < 6000) Sleep(10);
        elapsed = GetTickCount() - started;
    }
    step(5, "play", written && all_done(headers, 4) && wait_count(&done, 4, 1000) && elapsed >= 1800 && elapsed < 5000,
         (long)elapsed);

    /* 6: the position is the samples played, 88200; a TIME_MS request is
     * answered in bytes (Windows keeps only samples and bytes). */
    MMTIME samples, millis, bytes;
    samples.wType = TIME_SAMPLES;
    millis.wType = TIME_MS;
    bytes.wType = TIME_BYTES;
    MMRESULT p1 = waveOutGetPosition(device, &samples, sizeof samples);
    waveOutGetPosition(device, &millis, sizeof millis);
    waveOutGetPosition(device, &bytes, sizeof bytes);
    step(6, "position",
         p1 == 0 && samples.wType == TIME_SAMPLES && samples.u.sample <= 88200 && samples.u.sample >= 88200 - 441 &&
             millis.wType == TIME_BYTES && millis.u.cb == samples.u.sample * 2 && bytes.wType == TIME_BYTES &&
             bytes.u.cb == samples.u.sample * 2,
         (long)samples.u.sample);

    /* 7: unprepare and close: WOM_CLOSE. */
    int unprepared = 1;
    for (int i = 0; i < 4; i++)
        if (waveOutUnprepareHeader(device, &headers[i], sizeof headers[i]) != 0 || (headers[i].dwFlags & WHDR_PREPARED))
            unprepared = 0;
    MMRESULT c = waveOutClose(device);
    step(7, "close", unprepared && c == 0 && wait_count(&closed, 1, 1000), (long)c);

    /* 8: closing or unpreparing while playing is refused; waveOutReset
     * returns the buffer at once and rewinds. */
    done = 0;
    device = NULL;
    MMRESULT o2 = waveOutOpen(&device, WAVE_MAPPER, &f, (DWORD_PTR)wave_proc, 0, CALLBACK_FUNCTION);
    WAVEHDR one;
    memset(&one, 0, sizeof one);
    one.lpData = (LPSTR)tone660;
    one.dwBufferLength = 4 * HALF * sizeof(short);
    waveOutPrepareHeader(device, &one, sizeof one);
    MMRESULT w = waveOutWrite(device, &one, sizeof one);
    Sleep(200);
    MMRESULT busy_close = waveOutClose(device);
    MMRESULT busy_unprepare = waveOutUnprepareHeader(device, &one, sizeof one);
    MMRESULT reset = waveOutReset(device);
    int returned = (one.dwFlags & WHDR_DONE) && !(one.dwFlags & WHDR_INQUEUE);
    samples.wType = TIME_SAMPLES;
    waveOutGetPosition(device, &samples, sizeof samples);
    int callback = wait_count(&done, 1, 1000);
    MMRESULT unprepare = waveOutUnprepareHeader(device, &one, sizeof one);
    MMRESULT c2 = waveOutClose(device);
    step(8, "reset",
         o2 == 0 && w == 0 && busy_close == WAVERR_STILLPLAYING && busy_unprepare == WAVERR_STILLPLAYING && reset == 0 &&
             returned && samples.u.sample == 0 && callback && unprepare == 0 && c2 == 0,
         (long)busy_close * 100 + (long)busy_unprepare);

    /* 9: an event callback is signalled on open and when a buffer is done. */
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    device = NULL;
    MMRESULT o3 = waveOutOpen(&device, WAVE_MAPPER, &f, (DWORD_PTR)event, 0, CALLBACK_EVENT);
    DWORD on_open = WaitForSingleObject(event, 1000);
    memset(&one, 0, sizeof one);
    one.lpData = (LPSTR)tone660;
    one.dwBufferLength = 4410 * sizeof(short);
    waveOutPrepareHeader(device, &one, sizeof one);
    waveOutWrite(device, &one, sizeof one);
    DWORD on_done = WaitForSingleObject(event, 2000);
    int event_done = (one.dwFlags & WHDR_DONE) != 0;
    waveOutUnprepareHeader(device, &one, sizeof one);
    MMRESULT c3 = waveOutClose(device);
    CloseHandle(event);
    step(9, "event-callback", o3 == 0 && on_open == WAIT_OBJECT_0 && on_done == WAIT_OBJECT_0 && event_done && c3 == 0,
         (long)on_done);

    /* 10: PlaySound plays a WAV image from memory synchronously: one second
     * of 880 Hz at 22.05 kHz. */
    enum { RATE = 22050 };
    BYTE *image = allocate(44 + RATE * 2);
    BOOL played = FALSE;
    elapsed = 0;
    if (image) {
        DWORD data = RATE * 2;
        DWORD header[11] = {0x46464952, 36 + data, 0x45564157, 0x20746D66, 16, 0x00010001, RATE, RATE * 2, 0x00100002,
                            0x61746164, data};
        for (int i = 0; i < 44; i++) image[i] = ((BYTE *)header)[i];
        tone((short *)(image + 44), RATE, RATE, 880);
        started = GetTickCount();
        played = PlaySoundW((LPCWSTR)image, NULL, SND_MEMORY | SND_SYNC | SND_NODEFAULT);
        elapsed = GetTickCount() - started;
    }
    step(10, "playsound-memory", played && elapsed >= 900 && elapsed < 3000, (long)elapsed);

    /* 11: system sounds by alias and MessageBeep; a missing file fails
     * without the default sound; PlaySound(NULL) stops. */
    BOOL missing = PlaySoundW(L"C:\\no-such-sound.wav", NULL, SND_FILENAME | SND_NODEFAULT | SND_SYNC);
    BOOL alias = TRUE, beep = TRUE;
    if (!silent) {
        alias = PlaySoundW(L"SystemAsterisk", NULL, SND_ALIAS | SND_ASYNC | SND_NODEFAULT);
        Sleep(400);
        beep = MessageBeep(MB_ICONASTERISK);
    }
    BOOL stopped = PlaySoundW(NULL, NULL, 0);
    step(11, "system-sounds", !missing && alias && beep && stopped, silent ? 0 : 1);

    /* 12: error texts and handle validation. */
    wchar_t text[MAXERRORLENGTH];
    MMRESULT t1 = waveOutGetErrorTextW(MMSYSERR_NOERROR, text, MAXERRORLENGTH);
    int text_ok = t1 == 0 && wide_equal(text, L"The specified command was carried out.");
    MMRESULT t2 = waveOutGetErrorTextW(9999, text, MAXERRORLENGTH);
    MMRESULT bad = waveOutClose((HWAVEOUT)(ULONG_PTR)0x1234);
    step(12, "errors", text_ok && t2 == MMSYSERR_BADERRNUM && bad == MMSYSERR_INVALHANDLE, (long)bad);

    /* 13: multimedia timers: the periodic callback runs about every 10 ms. */
    TIMECAPS caps;
    MMRESULT dc = timeGetDevCaps(&caps, sizeof caps);
    MMRESULT begin = timeBeginPeriod(1);
    DWORD before = timeGetTime();
    UINT timer = timeSetEvent(10, 1, timer_proc, 0, TIME_PERIODIC | TIME_KILL_SYNCHRONOUS);
    Sleep(300);
    MMRESULT kill = timeKillEvent(timer);
    LONG fired = ticks;
    Sleep(50);
    DWORD spent = timeGetTime() - before;
    timeEndPeriod(1);
    step(13, "timers",
         dc == 0 && caps.wPeriodMin == 1 && caps.wPeriodMax == 1000000 && begin == 0 && timer != 0 && kill == 0 &&
             fired >= 15 && fired <= 40 && ticks == fired && spent >= 340,
         (long)fired);

    /* 14: two devices play at once, 440 Hz and 1000 Hz for a second each;
     * the capture must hold both in the same instants. */
    enum { SECOND = 44100 };
    short *mix = allocate(2 * SECOND * sizeof(short));
    HWAVEOUT pair[2] = {NULL, NULL};
    WAVEHDR mixed[2];
    memset(mixed, 0, sizeof mixed);
    int both = mix != NULL;
    started = GetTickCount();
    for (int i = 0; i < 2 && both; i++) {
        tone(mix + i * SECOND, SECOND, 44100, i == 0 ? 440 : 1000);
        mixed[i].lpData = (LPSTR)(mix + i * SECOND);
        mixed[i].dwBufferLength = SECOND * sizeof(short);
        both = waveOutOpen(&pair[i], WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL) == 0 &&
               waveOutPrepareHeader(pair[i], &mixed[i], sizeof mixed[i]) == 0;
    }
    for (int i = 0; i < 2 && both; i++) both = waveOutWrite(pair[i], &mixed[i], sizeof mixed[i]) == 0;
    elapsed = 0;
    if (both) {
        while (!all_done(mixed, 2) && GetTickCount() - started < 4000) Sleep(10);
        elapsed = GetTickCount() - started;
    }
    for (int i = 0; i < 2; i++) {
        if (!pair[i]) continue;
        waveOutUnprepareHeader(pair[i], &mixed[i], sizeof mixed[i]);
        if (waveOutClose(pair[i]) != 0) both = 0;
    }
    step(14, "mixed", both && all_done(mixed, 2) && elapsed >= 900 && elapsed < 3000, (long)elapsed);

    /* 15: an underrun: a quarter second of 550 Hz, silence while the
     * device starves, another quarter second; the position counts only
     * what played. */
    enum { QUARTER = 11025 };
    short *burst = allocate(QUARTER * sizeof(short));
    WAVEHDR parts[2];
    memset(parts, 0, sizeof parts);
    device = NULL;
    int resumed = burst != NULL && waveOutOpen(&device, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL) == 0;
    int starved = 0;
    if (resumed) {
        tone(burst, QUARTER, 44100, 550);
        for (int i = 0; i < 2; i++) {
            parts[i].lpData = (LPSTR)burst;
            parts[i].dwBufferLength = QUARTER * sizeof(short);
            waveOutPrepareHeader(device, &parts[i], sizeof parts[i]);
        }
        resumed = waveOutWrite(device, &parts[0], sizeof parts[0]) == 0;
        Sleep(700);
        starved = (parts[0].dwFlags & WHDR_DONE) != 0;
        resumed = resumed && waveOutWrite(device, &parts[1], sizeof parts[1]) == 0;
        started = GetTickCount();
        while (resumed && !all_done(parts, 2) && GetTickCount() - started < 3000) Sleep(10);
    }
    memset(&samples, 0, sizeof samples);
    samples.wType = TIME_SAMPLES;
    if (device) {
        waveOutGetPosition(device, &samples, sizeof samples);
        for (int i = 0; i < 2; i++) waveOutUnprepareHeader(device, &parts[i], sizeof parts[i]);
        if (waveOutClose(device) != 0) resumed = 0;
    }
    step(15, "underrun",
         resumed && starved && all_done(parts, 2) && samples.u.sample <= 2 * QUARTER && samples.u.sample >= 2 * QUARTER - 441,
         (long)samples.u.sample);

    put(failed_step ? "SOUND-A FAILED\r\n" : "SOUND-A PASSED\r\n");
    ExitProcess(failed_step ? (UINT)failed_step : 0);
}

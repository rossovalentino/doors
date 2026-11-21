/* MIDI-A probe (P15): an ordinary Windows console program (Windows SDK,
 * no C runtime) that uses the rest of Windows Multimedia: the MIDI
 * synthesizer through midiOut and midiStream, MCI command strings for a
 * wave file and a MIDI file, and the mixer API.
 *
 *   midiprobe.exe [silent]
 *
 * The acceptance run finds four tones in the guest's captured output:
 * 440 Hz (A4, square lead through midiOutShortMsg, one second), 659 Hz
 * (E5, saw lead through a MIDI stream, two beats at 120 bpm), 587 Hz (a
 * one-second wave file played by MCI) and 1047 Hz (C6 from a MIDI file
 * played by the MCI sequencer). "silent" sets channel volume 0 (and writes
 * silent files) and leaves every volume alone, so the same binary runs on
 * Windows as reference without making a sound or changing a setting.
 *
 * One "MIDI-A <step> ..." line per check; exit 0 only when all passed. */
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
static long wide_number(const wchar_t *s) {
    long v = 0;
    while (*s >= L'0' && *s <= L'9') v = v * 10 + (*s++ - L'0');
    return v;
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("MIDI-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static void finish(void) {
    put(failed_step ? "MIDI-A FAILED\r\n" : "MIDI-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}
static int silent;

/* Callback messages, counted. */
static volatile LONG opened, closed, done, positioned;
static void CALLBACK midi_callback(HMIDIOUT device, UINT message, DWORD_PTR instance, DWORD_PTR a, DWORD_PTR b) {
    (void)device;
    (void)instance;
    (void)a;
    (void)b;
    if (message == MOM_OPEN) InterlockedIncrement(&opened);
    if (message == MOM_CLOSE) InterlockedIncrement(&closed);
    if (message == MOM_DONE) InterlockedIncrement(&done);
    if (message == MOM_POSITIONCB) InterlockedIncrement(&positioned);
}
static int wait_for(volatile LONG *counter, LONG target, DWORD millis) {
    DWORD start = GetTickCount();
    while (*counter < target) {
        if (GetTickCount() - start > millis) return 0;
        Sleep(10);
    }
    return 1;
}
static DWORD short_message(BYTE status, BYTE a, BYTE b) {
    return (DWORD)status | (DWORD)a << 8 | (DWORD)b << 16;
}
static MCIERROR mci(const wchar_t *command, wchar_t *reply) {
    reply[0] = 0;
    return mciSendStringW(command, reply, 128, NULL);
}
static void write_file(const wchar_t *path, const void *data, DWORD bytes) {
    HANDLE f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    DWORD written;
    WriteFile(f, data, bytes, &written, NULL);
    CloseHandle(f);
}
/* tone.wav: one second of 587 Hz, 16-bit mono at 22050 Hz (a triangle). */
static BYTE wav[44 + 44100];
static void make_wave(void) {
    DWORD data = 44100;
    BYTE *p = wav;
    memset(wav, 0, sizeof wav);
    p[0] = 'R', p[1] = 'I', p[2] = 'F', p[3] = 'F';
    *(DWORD *)(p + 4) = 36 + data;
    p[8] = 'W', p[9] = 'A', p[10] = 'V', p[11] = 'E';
    p[12] = 'f', p[13] = 'm', p[14] = 't', p[15] = ' ';
    *(DWORD *)(p + 16) = 16;
    *(WORD *)(p + 20) = 1;
    *(WORD *)(p + 22) = 1;
    *(DWORD *)(p + 24) = 22050;
    *(DWORD *)(p + 28) = 44100;
    *(WORD *)(p + 32) = 2;
    *(WORD *)(p + 34) = 16;
    p[36] = 'd', p[37] = 'a', p[38] = 't', p[39] = 'a';
    *(DWORD *)(p + 40) = data;
    short *samples = (short *)(p + 44);
    unsigned int phase = 0, increment = (unsigned int)(((unsigned __int64)587 << 32) / 22050);
    for (int i = 0; i < 22050; i++) {
        int q = (int)(phase >> 16);
        int v = q < 32768 ? q - 16384 : 49151 - q;
        samples[i] = silent ? 0 : (short)(v / 2);
        phase += increment;
    }
}
/* song.mid: format 0, 96 ticks per quarter, square lead (program 80)
 * playing C6 for two beats at 120 bpm (one second). */
static BYTE song[] = {
    'M', 'T', 'h', 'd', 0, 0, 0, 6, 0, 0, 0, 1, 0, 96,
    'M', 'T', 'r', 'k', 0, 0, 0, 27,
    0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20, /* tempo 500000 */
    0, 0xC0, 80,                         /* program: square lead */
    0, 0xB0, 7, 100,                     /* channel volume (0 when silent) */
    0, 0x90, 84, 110,                    /* C6 on */
    0x81, 0x40, 0x80, 84, 0,             /* 192 ticks later: off */
    0, 0xFF, 0x2F, 0,                    /* end of track */
};

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    {
        const wchar_t *p = GetCommandLineW();
        while (*p) {
            if (p[0] == L's' && wide_equal(p, L"silent")) silent = 1;
            p++;
        }
    }
    BYTE volume = silent ? 0 : 100;

    /* 1. The synthesizer. */
    MIDIOUTCAPSW caps;
    memset(&caps, 0, sizeof caps);
    MMRESULT r = midiOutGetDevCapsW(0, &caps, sizeof caps);
    step(1, "caps", midiOutGetNumDevs() == 1 && r == MMSYSERR_NOERROR && caps.wTechnology == MOD_SWSYNTH && caps.wVoices == 32 && caps.wNotes == 32 && caps.wChannelMask == 0xFFFF && (caps.dwSupport & MIDICAPS_VOLUME), caps.wTechnology);

    /* 2. The MIDI mapper; no MIDI input or auxiliary device. */
    MIDIOUTCAPSW mapper;
    memset(&mapper, 0, sizeof mapper);
    r = midiOutGetDevCapsW(MIDI_MAPPER, &mapper, sizeof mapper);
    step(2, "mapper", r == MMSYSERR_NOERROR && mapper.wTechnology == MOD_MAPPER && (mapper.dwSupport & MIDICAPS_STREAM) && midiInGetNumDevs() == 0 && auxGetNumDevs() == 0, r);

    /* 3. Open with a function callback; a second open is refused. */
    HMIDIOUT device = NULL, second = NULL;
    r = midiOutOpen(&device, 0, (DWORD_PTR)midi_callback, 0, CALLBACK_FUNCTION);
    MMRESULT again = midiOutOpen(&second, 0, 0, 0, CALLBACK_NULL);
    step(3, "open", r == MMSYSERR_NOERROR && wait_for(&opened, 1, 1000) && again == MMSYSERR_ALLOCATED, again);
    if (r != MMSYSERR_NOERROR) finish();

    /* 4. A4 on a square lead for one second. */
    MMRESULT sent = midiOutShortMsg(device, short_message(0xB0, 7, volume));
    sent |= midiOutShortMsg(device, short_message(0xC0, 80, 0));
    sent |= midiOutShortMsg(device, short_message(0x90, 69, 110));
    Sleep(1000);
    sent |= midiOutShortMsg(device, short_message(0x80, 69, 0));
    step(4, "notes", sent == MMSYSERR_NOERROR, sent);

    /* 5. A system exclusive message (GM System On). */
    static unsigned char reset[] = {0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7};
    MIDIHDR header;
    memset(&header, 0, sizeof header);
    header.lpData = (LPSTR)reset;
    header.dwBufferLength = sizeof reset;
    MMRESULT unprepared = midiOutLongMsg(device, &header, sizeof header);
    r = midiOutPrepareHeader(device, &header, sizeof header);
    LONG before = done;
    if (r == MMSYSERR_NOERROR) r = midiOutLongMsg(device, &header, sizeof header);
    int reported = wait_for(&done, before + 1, 1000) && (header.dwFlags & MHDR_DONE);
    MMRESULT unprepare = midiOutUnprepareHeader(device, &header, sizeof header);
    step(5, "sysex", unprepared == MIDIERR_UNPREPARED && r == MMSYSERR_NOERROR && reported && unprepare == MMSYSERR_NOERROR, unprepared);

    /* 6. The device volume (left and right words). */
    DWORD level = 0;
    r = midiOutGetVolume(device, &level);
    int volumes = r == MMSYSERR_NOERROR && level == 0xFFFFFFFF;
    if (!silent && volumes) {
        DWORD half = 0;
        volumes = midiOutSetVolume(device, 0x80008000) == MMSYSERR_NOERROR && midiOutGetVolume(device, &half) == MMSYSERR_NOERROR && half == 0x80008000 && midiOutSetVolume(device, 0xFFFFFFFF) == MMSYSERR_NOERROR;
    }
    step(6, "volume", volumes, r);

    /* 7. Close: MOM_CLOSE, and the handle is gone. */
    r = midiOutClose(device);
    step(7, "close", r == MMSYSERR_NOERROR && wait_for(&closed, 1, 1000) && midiOutClose(device) == MMSYSERR_INVALHANDLE, r);

    /* 8. A MIDI stream: its default division and tempo. */
    HMIDISTRM stream = NULL;
    UINT id = 0;
    opened = closed = done = 0;
    r = midiStreamOpen(&stream, &id, 1, (DWORD_PTR)midi_callback, 0, CALLBACK_FUNCTION);
    MIDIPROPTIMEDIV division;
    MIDIPROPTEMPO tempo;
    memset(&division, 0, sizeof division);
    memset(&tempo, 0, sizeof tempo);
    division.cbStruct = sizeof division;
    tempo.cbStruct = sizeof tempo;
    MMRESULT got = r == MMSYSERR_NOERROR ? midiStreamProperty(stream, (LPBYTE)&division, MIDIPROP_GET | MIDIPROP_TIMEDIV) : r;
    if (got == MMSYSERR_NOERROR) got = midiStreamProperty(stream, (LPBYTE)&tempo, MIDIPROP_GET | MIDIPROP_TEMPO);
    int defaults = got == MMSYSERR_NOERROR && division.dwTimeDiv == 24 && tempo.dwTempo == 500000;
    division.dwTimeDiv = 96;
    if (defaults) got = midiStreamProperty(stream, (LPBYTE)&division, MIDIPROP_SET | MIDIPROP_TIMEDIV);
    step(8, "stream-open", r == MMSYSERR_NOERROR && defaults && got == MMSYSERR_NOERROR && wait_for(&opened, 1, 1000), (long)division.dwTimeDiv);
    if (r != MMSYSERR_NOERROR) finish();

    /* 9. E5 on a saw lead for two beats, then a callback event. */
    static DWORD events[] = {
        0, 0, MEVT_SHORTMSG << 24 | 0x000007B1,         /* channel 2 volume (set below) */
        0, 0, MEVT_SHORTMSG << 24 | 0x000051C1,         /* program 81: saw lead */
        0, 0, MEVT_SHORTMSG << 24 | 0x006E4C91,         /* E5 on */
        192, 0, MEVT_SHORTMSG << 24 | 0x00004C81,       /* two beats: off */
        0, 0, MEVT_F_CALLBACK | MEVT_NOP << 24,          /* position callback */
    };
    events[2] |= (DWORD)volume << 16;
    MIDIHDR buffer;
    memset(&buffer, 0, sizeof buffer);
    buffer.lpData = (LPSTR)events;
    buffer.dwBufferLength = sizeof events;
    buffer.dwBytesRecorded = sizeof events;
    r = midiOutPrepareHeader((HMIDIOUT)stream, &buffer, sizeof buffer);
    if (r == MMSYSERR_NOERROR) r = midiStreamOut(stream, &buffer, sizeof buffer);
    if (r == MMSYSERR_NOERROR) r = midiStreamRestart(stream);
    int finished = r == MMSYSERR_NOERROR && wait_for(&done, 1, 4000) && (buffer.dwFlags & MHDR_DONE);
    MMTIME ms, ticks;
    ms.wType = TIME_MS;
    ticks.wType = TIME_TICKS;
    midiStreamPosition(stream, &ms, sizeof ms);
    midiStreamPosition(stream, &ticks, sizeof ticks);
    step(9, "stream-play", finished && positioned == 1 && ms.wType == TIME_MS && ms.u.ms >= 900 && ticks.wType == TIME_TICKS && ticks.u.ticks >= 192, (long)ms.u.ms);

    /* 10. Stop and close. */
    MMRESULT stopped = midiStreamStop(stream);
    MMRESULT unprepared_buffer = midiOutUnprepareHeader((HMIDIOUT)stream, &buffer, sizeof buffer);
    r = midiStreamClose(stream);
    step(10, "stream-close", stopped == MMSYSERR_NOERROR && unprepared_buffer == MMSYSERR_NOERROR && r == MMSYSERR_NOERROR && wait_for(&closed, 1, 1000), r);

    /* 11. MCI: a wave file. */
    wchar_t reply[128];
    make_wave();
    write_file(L"midiprobe-tone.wav", wav, sizeof wav);
    MCIERROR e = mci(L"open midiprobe-tone.wav type waveaudio alias tone", reply);
    long opened_id = wide_number(reply);
    MCIERROR e2 = mci(L"status tone length", reply);
    long length_ms = wide_number(reply);
    MCIERROR e3 = mci(L"play tone wait", reply);
    MCIERROR e4 = mci(L"status tone mode", reply);
    int stopped_mode = wide_equal(reply, L"stopped");
    MCIERROR e5 = mci(L"status tone position", reply);
    long position = wide_number(reply);
    MCIERROR e6 = mci(L"close tone", reply);
    step(11, "mci-wave", !e && opened_id >= 1 && !e2 && length_ms == 1000 && !e3 && !e4 && stopped_mode && !e5 && position == 1000 && !e6, e ? (long)e : length_ms);

    /* 12. MCI: a MIDI file through the sequencer. */
    song[sizeof song - 14] = volume; /* the channel volume value */
    write_file(L"midiprobe-song.mid", song, sizeof song);
    /* The sequencer counts in song pointer units (sixteenth notes) until
     * told otherwise: two beats are 8. */
    e = mci(L"open midiprobe-song.mid type sequencer alias song", reply);
    MCIERROR format = mci(L"status song time format", reply);
    int pointer = wide_equal(reply, L"song pointer");
    e2 = mci(L"status song length", reply);
    long sixteenths = wide_number(reply);
    MCIERROR formatted = mci(L"set song time format milliseconds", reply);
    MCIERROR e7 = mci(L"status song length", reply);
    length_ms = wide_number(reply);
    e3 = mci(L"play song wait", reply);
    e4 = mci(L"status song mode", reply);
    stopped_mode = wide_equal(reply, L"stopped");
    e6 = mci(L"close song", reply);
    step(12, "mci-sequencer", !e && !format && pointer && !e2 && sixteenths == 8 && !formatted && !e7 && length_ms == 1000 && !e3 && !e4 && stopped_mode && !e6, length_ms);

    /* 13. MCI errors and their texts. */
    MCIERROR missing = mci(L"open c:\\nonexistent.wav alias missing", reply);
    MCIERROR unknown = mci(L"status nosuch mode", reply);
    MCIERROR nameless = mci(L"bogus", reply);
    wchar_t text[256];
    mciGetErrorStringW(unknown, text, 256);
    step(13, "mci-errors", missing == MCIERR_FILE_NOT_FOUND && unknown == MCIERR_INVALID_DEVICE_NAME && nameless == MCIERR_MISSING_DEVICE_NAME && wide_equal(text, L"The specified device is not open or is not recognized by MCI."), unknown);

    /* 14. The mixer: the speakers' master volume line. */
    MIXERLINEW line;
    memset(&line, 0, sizeof line);
    line.cbStruct = sizeof line;
    line.dwComponentType = MIXERLINE_COMPONENTTYPE_DST_SPEAKERS;
    r = mixerGetLineInfoW((HMIXEROBJ)0, &line, MIXER_OBJECTF_MIXER | MIXER_GETLINEINFOF_COMPONENTTYPE);
    MIXERCONTROLW control;
    MIXERLINECONTROLSW controls;
    memset(&controls, 0, sizeof controls);
    controls.cbStruct = sizeof controls;
    controls.dwLineID = line.dwLineID;
    controls.dwControlType = MIXERCONTROL_CONTROLTYPE_VOLUME;
    controls.cControls = 1;
    controls.cbmxctrl = sizeof control;
    controls.pamxctrl = &control;
    MMRESULT found = r == MMSYSERR_NOERROR ? mixerGetLineControlsW((HMIXEROBJ)0, &controls, MIXER_OBJECTF_MIXER | MIXER_GETLINECONTROLSF_ONEBYTYPE) : r;
    MIXERCONTROLDETAILS details;
    MIXERCONTROLDETAILS_UNSIGNED value;
    memset(&details, 0, sizeof details);
    details.cbStruct = sizeof details;
    details.dwControlID = control.dwControlID;
    details.cChannels = 1;
    details.cbDetails = sizeof value;
    details.paDetails = &value;
    MMRESULT read = found == MMSYSERR_NOERROR ? mixerGetControlDetailsW((HMIXEROBJ)0, &details, MIXER_OBJECTF_MIXER | MIXER_GETCONTROLDETAILSF_VALUE) : found;
    int mixer_ok = mixerGetNumDevs() >= 1 && r == MMSYSERR_NOERROR && line.cControls == 2 && wide_equal(line.szName, L"Master Volume") && found == MMSYSERR_NOERROR && control.Bounds.dwMaximum == 65535 && read == MMSYSERR_NOERROR;
    if (!silent && mixer_ok) {
        /* In the guest: 70% through the mixer, read back, restored. */
        DWORD original = value.dwValue;
        value.dwValue = 45875;
        MMRESULT set = mixerSetControlDetails((HMIXEROBJ)0, &details, MIXER_OBJECTF_MIXER | MIXER_SETCONTROLDETAILSF_VALUE);
        /* Long enough for the shell (it polls once a second) to see it. */
        Sleep(2500);
        value.dwValue = 0;
        mixerGetControlDetailsW((HMIXEROBJ)0, &details, MIXER_OBJECTF_MIXER | MIXER_GETCONTROLDETAILSF_VALUE);
        DWORD readback = value.dwValue;
        value.dwValue = original;
        mixerSetControlDetails((HMIXEROBJ)0, &details, MIXER_OBJECTF_MIXER | MIXER_SETCONTROLDETAILSF_VALUE);
        mixer_ok = set == MMSYSERR_NOERROR && readback >= 45000 && readback <= 46500;
    }
    step(14, "mixer", mixer_ok, (long)line.cConnections);
    DeleteFileW(L"midiprobe-tone.wav");
    DeleteFileW(L"midiprobe-song.mid");
    finish();
}

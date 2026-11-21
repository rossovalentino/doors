/* Doors OS P13 fixture F1 (wingui-selftest.exe): a deterministic, self-driving
 * Win32 GUI self-test.
 *
 * Ordinary Windows SDK C without a CRT, built twice with the pinned MSVC
 * toolchain. It has no Door headers, imports or conditional paths: the same
 * executable bytes run on the Windows reference and in Door. Every input is a
 * message posted or sent to this process's own windows. It never injects
 * system input, changes settings or touches the clipboard.
 *
 * Modes (one exact argument):
 *   (none)    self-test; prints one WINDOWS_GUI_OK line and exits 75
 *   --reopen  reopens the saved file through GetOpenFileNameW; exits 76
 *   --probe   runs the same sequence, prints every measured value instead of
 *             comparing it, writes wingui-raster.bin and exits 77
 * A failed check prints one WINDOWS_GUI_FAIL line and exits with its code.
 * Every line goes to standard output (when launched with one, such as from a
 * console) and to wingui-selftest[-reopen|-probe].results in the working
 * directory. The common dialogs pass OFN_DONTADDTORECENT so a reference run
 * adds nothing to the user's recent documents.
 */
#define UNICODE
#define _UNICODE
#define WINVER 0x0a00
#define _WIN32_WINNT 0x0a00
#include <windows.h>
#include <commdlg.h>
#include <dlgs.h>

#define EXIT_OK 75
#define EXIT_REOPEN 76
#define EXIT_PROBE 77

#define ID_FILE_RUN 101
#define ID_FILE_EXIT 102
#define ID_HELP_ABOUT 103
#define ID_BUTTON 201
#define ID_STATIC 202
#define ID_EDIT 203

#define WM_NESTED_OUTER (WM_APP + 1)
#define WM_NESTED_INNER (WM_APP + 2)
#define WM_NESTED_POSTED (WM_APP + 3)
#define WM_CROSS_REQUEST (WM_APP + 4)
#define WM_CROSS_CALLBACK (WM_APP + 5)
#define WM_HUNG_PROBE (WM_APP + 6)
#define WM_CROSS_POSTED (WM_APP + 7)
#define WM_ORDER_POSTED (WM_APP + 10)
#define WM_ORDER_SENT (WM_APP + 11)
#define WM_SENTINEL (WM_APP + 12)

/* Last-error sentinel stored before every measured failing call. */
#define SENTINEL 0x0badu
#define RASTER_W 128
#define RASTER_H 64

static const WCHAR main_class[] = L"DoorsGuiSelftest";
static const WCHAR worker_class[] = L"DoorsGuiWorker";
static const WCHAR hung_class[] = L"DoorsGuiHung";
static const WCHAR reject_class[] = L"DoorsGuiReject";
static const WCHAR missing_class[] = L"DoorsGuiMissingClass";
static const WCHAR title[] = L"Doors GUI self-test \x03bb";
static const WCHAR box_text[] = L"Doors self-test message \x03bb \x9580";
static const WCHAR file_name[] = L"wingui-selftest-\x03bb.txt";
static const WCHAR long_name[] = L"wingui-selftest-long-name.txt";
static const WCHAR file_text[] = L"\xfeff" L"Doors GUI self-test\r\n\x03bb \x9580 \x00e9\r\n";
static const WCHAR stage_one[] = L"Doors\r\nx\x00e9";
static const WCHAR stage_two[] = L"Doors\r\nZ\x00e9";

static HINSTANCE instance;
static BOOL probing;
static HWND main_hwnd, button_hwnd, static_hwnd, edit_hwnd;
static HMENU main_menu, file_menu, help_menu;
static WNDPROC button_original, static_original, edit_original;
static DWORD main_thread;
static const int create_marker = 0x5a;
static volatile LONG create_checks;
static volatile LONG paint_count;
static volatile LONG sentinel_seen;
static volatile LONG en_change;
static volatile LONG clicks;
static volatile LONG menu_commands;
static volatile LONG accelerator_commands;
static volatile LONG timer_ticks;
static volatile LONG box_checks;
static volatile LONG box_attempts;
static volatile LONG nested_posted;
static volatile LONG cross_posted;
static volatile LONG cross_order[8];
static volatile LONG cross_steps;
static volatile LONG cross_main_ok;
static volatile LONG cross_insend_worker;
static volatile LONG cross_insend_main;
static volatile LONG nested_insend;
static volatile LONG nested_order[4];
static volatile LONG nested_steps;
static int reject_mode;
static int hook_action;
static WCHAR hook_path[MAX_PATH + 64];
static volatile LONG hook_initdone;
static volatile LONG hook_fileok;
static volatile LONG hook_posts;
static HWND worker_hwnd, hung_hwnd;
static HANDLE worker_ready, hung_ready, hung_release;
static DWORD worker_thread;
static volatile LONG hung_delivered;
static DWORD raster_crc;
static WCHAR saved_path[MAX_PATH + 64];
static DWORD raster_pixels[RASTER_W * RASTER_H];

/* ---- no-CRT helpers -------------------------------------------------- */

static void zero(void *memory, SIZE_T count)
{
    volatile BYTE *bytes = (volatile BYTE *)memory;
    while (count--)
        *bytes++ = 0;
}

static DWORD wlen(const WCHAR *text)
{
    DWORD count = 0;
    while (text[count])
        ++count;
    return count;
}

static BOOL weq(const WCHAR *a, const WCHAR *b)
{
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return *a == *b;
}

static WCHAR fold(WCHAR c)
{
    return (WCHAR)(c >= L'a' && c <= L'z' ? c - 32 : c);
}

static BOOL weq_ascii_nocase(const WCHAR *a, const WCHAR *b)
{
    while (*a && fold(*a) == fold(*b)) {
        ++a;
        ++b;
    }
    return *a == *b;
}

static void wappend(WCHAR *buffer, DWORD capacity, const WCHAR *text)
{
    DWORD at = wlen(buffer);
    while (*text && at + 1 < capacity)
        buffer[at++] = *text++;
    buffer[at] = 0;
}

static DWORD crc_byte(DWORD crc, BYTE value)
{
    int bit;
    crc ^= value;
    for (bit = 0; bit < 8; ++bit)
        crc = (crc >> 1) ^ (0xedb88320u & (0u - (crc & 1u)));
    return crc;
}

/* ---- output ------------------------------------------------------------ */

static HANDLE out_handle;
static HANDLE results_handle;
static char line[2048];
static DWORD line_len;

static void put_char(char c)
{
    if (line_len < sizeof(line))
        line[line_len++] = c;
}

static void put(const char *text)
{
    while (*text)
        put_char(*text++);
}

static void put_dec(DWORD value)
{
    char digits[10];
    int count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    while (count)
        put_char(digits[--count]);
}

static void put_hex(DWORD value, int width)
{
    static const char hex[] = "0123456789abcdef";
    int shift;
    for (shift = (width - 1) * 4; shift >= 0; shift -= 4)
        put_char(hex[(value >> shift) & 15]);
}

static void write_all(HANDLE handle, DWORD count)
{
    DWORD written = 0;
    if (!handle || handle == INVALID_HANDLE_VALUE || !count)
        return;
    if (!WriteFile(handle, line, count, &written, NULL) || written != count)
        ExitProcess(90);
}

/* Every result line goes to standard output (when the process has one) and to
   the results file in the working directory. */
static void flush_line(void)
{
    DWORD count = line_len;
    line_len = 0;
    write_all(out_handle, count);
    write_all(results_handle, count);
}

static __declspec(noreturn) void fail(UINT code)
{
    DWORD error = GetLastError();
    line_len = 0;
    put("WINDOWS_GUI_FAIL code=");
    put_dec(code);
    put(" error=");
    put_dec(error);
    put("\r\n");
    flush_line();
    ExitProcess(code);
}

/* Probe mode names each stage before running it, so a hang is attributable. */
static void stage(const char *name)
{
    if (!probing)
        return;
    put("WINDOWS_GUI_STAGE ");
    put(name);
    put("\r\n");
    flush_line();
}

/* ---- measured values ------------------------------------------------------ */

typedef struct {
    const char *name;
    DWORD ret;
    DWORD error;
} Probe;

#define PROBE_MAX 48
static Probe probes[PROBE_MAX];
static DWORD probe_count;

static void probe(const char *name, DWORD ret, DWORD error)
{
    if (probe_count >= PROBE_MAX)
        fail(318);
    probes[probe_count].name = name;
    probes[probe_count].ret = ret;
    probes[probe_count].error = error;
    ++probe_count;
}

/* Windows 11 reference values, measured with --probe on the owner's machine
   (see the reference record); the order is the execution order. Several differ
   from the documentation: SetWindowTextW on a destroyed window returns TRUE with
   error 1400, an SMTO_ABORTIFHUNG timeout leaves the last error untouched, a
   timed-out send is never delivered late, and FNERR_BUFFERTOOSMALL is reported
   with a nonzero GetSaveFileNameW result. */
static const Probe expected_probes[] = {
    {"register_duplicate_class", 0, 1410},
    {"edit_en_change", 14, 0},
    {"timer_count", 3, 0},
    {"msgbox_result", IDCANCEL, 0},
    {"nested_insend", 0, 0},
    {"cross_thread_insend", 1, 1},
    {"destroy_other_thread", 0, 5},
    {"iswindow_destroyed", 0, SENTINEL},
    {"send_destroyed", 0, 1400},
    {"post_destroyed", 0, 1400},
    {"destroy_destroyed", 0, 1400},
    {"gettext_destroyed", 0, SENTINEL},
    {"settext_destroyed", 1, 1400},
    {"peek_destroyed", 0, 1400},
    {"send_timeout_destroyed", 0, 1400},
    {"create_unknown_class", 0, 1407},
    {"create_child_without_parent", 0, 1406},
    {"create_invalid_parent", 0, 1400},
    {"create_nccreate_false", 0, SENTINEL},
    {"create_create_minus_one", 0, 0},
    {"unregister_class_in_use", 0, 1412},
    {"unregister_missing_class", 0, 1411},
    {"is_hung_initial", 0, SENTINEL},
    {"send_timeout_busy", 0, 1460},
    {"send_timeout_busy_waited", 1, 0},
    {"is_hung_after", 1, SENTINEL},
    {"send_timeout_hung_abort", 0, SENTINEL},
    {"send_timeout_hung_fast", 1, 0},
    {"hung_late_delivery", 0, 0},
    {"save_struct_size", 0, 1},
    {"save_cancel", 0, 0},
    {"save_buffer_small", 1, 0x3003},
    {"save_buffer_small_reported", 1, 0},
    {"save_ok", 1, 0},
};

/* Ordered message traces: tag in the high byte, message (or event) below. */
#define TRACE_MAX 128
typedef struct {
    const char *name;
    DWORD count;
    DWORD entries[TRACE_MAX];
} Trace;
static Trace traces[6];
static DWORD trace_count;
static Trace *tracing;

#define T(tag, value) (((DWORD)(BYTE)(tag) << 24) | (DWORD)(value))
static void trace_add(char tag, DWORD value)
{
    if (tracing && tracing->count < TRACE_MAX)
        tracing->entries[tracing->count++] = T(tag, value & 0xffffffu);
}

static void trace_begin(const char *name)
{
    if (trace_count >= sizeof(traces) / sizeof(traces[0]))
        fail(319);
    tracing = &traces[trace_count++];
    tracing->name = name;
    tracing->count = 0;
}

static void trace_end(void)
{
    tracing = NULL;
}

/* PN(event, id): WM_PARENTNOTIFY for a child event (WM_CREATE/WM_DESTROY). */
#define PN(event, id) T('P', ((DWORD)(event) << 12) | (DWORD)(id))
static const DWORD expected_create[] = {
    T('M', WM_GETMINMAXINFO), T('M', WM_NCCREATE), T('M', WM_NCCALCSIZE), T('M', WM_CREATE),
    PN(WM_CREATE, ID_BUTTON), PN(WM_CREATE, ID_STATIC), PN(WM_CREATE, ID_EDIT),
};
static const DWORD expected_show[] = {
    T('M', WM_SHOWWINDOW), T('M', WM_WINDOWPOSCHANGING), T('M', WM_WINDOWPOSCHANGED),
    T('M', WM_SIZE), T('M', WM_MOVE), T('M', WM_PAINT),
};
static const DWORD expected_order[] = {
    T('O', 0xb03), T('O', 0xa01), T('O', 0xa02), T('M', WM_PAINT),
};
static const DWORD expected_reject_nc[] = {
    T('R', WM_GETMINMAXINFO), T('R', WM_NCCREATE), T('R', WM_NCDESTROY),
};
static const DWORD expected_reject_create[] = {
    T('R', WM_GETMINMAXINFO), T('R', WM_NCCREATE), T('R', WM_NCCALCSIZE), T('R', WM_CREATE),
    T('R', WM_DESTROY), T('R', WM_NCDESTROY),
};
static const DWORD expected_destroy[] = {
    T('M', WM_WINDOWPOSCHANGING), T('M', WM_WINDOWPOSCHANGED), T('M', WM_DESTROY),
    T('B', WM_DESTROY), T('S', WM_DESTROY), T('E', WM_DESTROY),
    T('B', WM_NCDESTROY), T('S', WM_NCDESTROY), T('E', WM_NCDESTROY), T('M', WM_NCDESTROY),
};

static void compare_trace(DWORD index, const DWORD *expected, DWORD count, UINT code)
{
    DWORD i;
    if (probing)
        return;
    if (index >= trace_count || traces[index].count != count)
        fail(code);
    for (i = 0; i < count; ++i)
        if (traces[index].entries[i] != expected[i])
            fail(code);
}

/* ---- message pumping ---------------------------------------------------------- */

static BOOL pump_once(DWORD wait)
{
    MSG message;
    if (PeekMessageW(&message, NULL, 0, 0, PM_REMOVE)) {
        if (message.message == WM_QUIT)
            fail(91);
        TranslateMessage(&message);
        DispatchMessageW(&message);
        return TRUE;
    }
    if (wait)
        MsgWaitForMultipleObjects(0, NULL, FALSE, wait, QS_ALLINPUT);
    return FALSE;
}

static void pump_until(volatile LONG *value, LONG target, DWORD timeout, UINT code)
{
    ULONGLONG deadline = GetTickCount64() + timeout;
    while (*value < target) {
        if (GetTickCount64() > deadline)
            fail(code);
        pump_once(5);
    }
}

static void pump_for(DWORD milliseconds)
{
    ULONGLONG deadline = GetTickCount64() + milliseconds;
    while (GetTickCount64() < deadline)
        pump_once(5);
}

/* Posted messages of one thread are retrieved in order: once the sentinel
   arrives, every message posted before it has been dispatched. */
static void drain(UINT code)
{
    sentinel_seen = 0;
    if (!PostMessageW(main_hwnd, WM_SENTINEL, 0, 0))
        fail(code);
    pump_until(&sentinel_seen, 1, 3000, code);
}

/* ---- drawing ------------------------------------------------------------------- */

/* The non-text scene shared by WM_PAINT and the memory-DC raster check. */
static void draw_scene(HDC dc)
{
    static const DWORD rops[16] = {
        SRCCOPY, SRCAND, SRCPAINT, SRCINVERT, NOTSRCCOPY, MERGEPAINT, PATINVERT, DSTINVERT,
        MERGECOPY, NOTSRCERASE, SRCERASE, PATPAINT, 0x00b8074a, 0x00e20746, PATCOPY, WHITENESS,
    };
    BITMAPINFO info;
    void *bits = NULL;
    HBITMAP source_bitmap;
    HDC source;
    HGDIOBJ old_source, old_pen, old_brush;
    HBRUSH back, fill, hatch, pattern, left, right;
    HPEN yellow, green;
    RECT rect;
    int i, x, y;

    zero(&info, sizeof(info));
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = 16;
    info.bmiHeader.biHeight = -16;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    source_bitmap = CreateDIBSection(NULL, &info, DIB_RGB_COLORS, &bits, NULL, 0);
    if (!source_bitmap || !bits)
        fail(140);
    for (y = 0; y < 16; ++y)
        for (x = 0; x < 16; ++x)
            ((DWORD *)bits)[y * 16 + x] = (((x >> 2) ^ (y >> 2)) & 1) ? 0x00ff00ffu : 0x0000c040u;
    source = CreateCompatibleDC(NULL);
    if (!source)
        fail(141);
    old_source = SelectObject(source, source_bitmap);

    back = CreateSolidBrush(RGB(0x20, 0x40, 0x60));
    fill = CreateSolidBrush(RGB(200, 30, 30));
    hatch = CreateHatchBrush(HS_DIAGCROSS, RGB(255, 128, 0));
    pattern = CreateSolidBrush(RGB(0x10, 0x80, 0xf0));
    left = CreateSolidBrush(RGB(0x33, 0x66, 0x99));
    right = CreateSolidBrush(RGB(0xcc, 0x99, 0x66));
    yellow = CreatePen(PS_SOLID, 1, RGB(255, 255, 0));
    green = CreatePen(PS_SOLID, 1, RGB(0, 255, 0));
    if (!back || !fill || !hatch || !pattern || !left || !right || !yellow || !green)
        fail(142);

    SetRect(&rect, 0, 0, RASTER_W, RASTER_H);
    if (!FillRect(dc, &rect, back))
        fail(143);
    old_pen = SelectObject(dc, yellow);
    old_brush = SelectObject(dc, fill);
    if (!Rectangle(dc, 4, 4, 40, 28))
        fail(144);
    SelectObject(dc, green);
    MoveToEx(dc, 2, 30, NULL);
    LineTo(dc, 126, 30);
    MoveToEx(dc, 64, 2, NULL);
    LineTo(dc, 64, 29);
    MoveToEx(dc, 44, 2, NULL);
    LineTo(dc, 62, 11);
    MoveToEx(dc, 70, 4, NULL);
    LineTo(dc, 90, 24);
    MoveToEx(dc, 92, 3, NULL);
    if (!LineTo(dc, 76, 27))
        fail(145);
    SetBkColor(dc, RGB(0x40, 0x00, 0x40));
    SelectObject(dc, hatch);
    if (!PatBlt(dc, 96, 4, 28, 24, PATCOPY))
        fail(146);
    SelectObject(dc, pattern);
    for (i = 0; i < 16; ++i) {
        x = (i % 8) * 16;
        y = 32 + (i / 8) * 16;
        SetRect(&rect, x, y, x + 8, y + 16);
        FillRect(dc, &rect, left);
        SetRect(&rect, x + 8, y, x + 16, y + 16);
        FillRect(dc, &rect, right);
        if (!BitBlt(dc, x, y, 16, 16, source, 0, 0, rops[i]))
            fail(147);
    }
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    SelectObject(source, old_source);
    DeleteDC(source);
    DeleteObject(source_bitmap);
    DeleteObject(back);
    DeleteObject(fill);
    DeleteObject(hatch);
    DeleteObject(pattern);
    DeleteObject(left);
    DeleteObject(right);
    DeleteObject(yellow);
    DeleteObject(green);
}

/* Renders the scene into a 128x64 top-down 32-bpp DIB section and returns the
   CRC-32 (IEEE) of each pixel's low 24 bits as four little-endian bytes. */
static DWORD render_raster(BOOL keep)
{
    BITMAPINFO info;
    void *bits = NULL;
    HBITMAP bitmap;
    HDC dc;
    HGDIOBJ old;
    DWORD crc = 0xffffffffu, i, pixel;

    zero(&info, sizeof(info));
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = RASTER_W;
    info.bmiHeader.biHeight = -RASTER_H;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    bitmap = CreateDIBSection(NULL, &info, DIB_RGB_COLORS, &bits, NULL, 0);
    dc = CreateCompatibleDC(NULL);
    if (!bitmap || !bits || !dc)
        fail(148);
    old = SelectObject(dc, bitmap);
    draw_scene(dc);
    if (GetPixel(dc, 10, 10) != RGB(200, 30, 30) || GetPixel(dc, 4, 4) != RGB(255, 255, 0)
        || GetPixel(dc, 4, 27) != RGB(255, 255, 0) || GetPixel(dc, 40, 10) != RGB(0x20, 0x40, 0x60)
        || GetPixel(dc, 10, 30) != RGB(0, 255, 0) || GetPixel(dc, 126, 30) != RGB(0x20, 0x40, 0x60))
        fail(149);
    GdiFlush();
    for (i = 0; i < RASTER_W * RASTER_H; ++i) {
        pixel = ((const DWORD *)bits)[i] & 0x00ffffffu;
        if (keep)
            raster_pixels[i] = pixel;
        crc = crc_byte(crc, (BYTE)pixel);
        crc = crc_byte(crc, (BYTE)(pixel >> 8));
        crc = crc_byte(crc, (BYTE)(pixel >> 16));
        crc = crc_byte(crc, 0);
    }
    SelectObject(dc, old);
    DeleteDC(dc);
    DeleteObject(bitmap);
    return ~crc;
}

static void paint_window(HWND hwnd)
{
    PAINTSTRUCT paint;
    POINT old;
    HDC dc = BeginPaint(hwnd, &paint);
    if (!dc)
        fail(150);
    SetViewportOrgEx(dc, 280, 150, &old);
    draw_scene(dc);
    SetViewportOrgEx(dc, old.x, old.y, NULL);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, RGB(0x10, 0x20, 0x80));
    if (!TextOutW(dc, 12, 160, L"Doors GDI \x03bb \x9580", 13))
        fail(151);
    EndPaint(hwnd, &paint);
    InterlockedIncrement(&paint_count);
}

/* ---- window procedures --------------------------------------------------------- */

static void track_main(UINT message, WPARAM wparam)
{
    switch (message) {
    case WM_GETMINMAXINFO: case WM_NCCREATE: case WM_NCCALCSIZE: case WM_CREATE:
    case WM_SHOWWINDOW: case WM_WINDOWPOSCHANGING: case WM_WINDOWPOSCHANGED:
    case WM_MOVE: case WM_SIZE: case WM_PAINT: case WM_DESTROY: case WM_NCDESTROY:
        trace_add('M', message);
        break;
    case WM_PARENTNOTIFY:
        if (LOWORD(wparam) == WM_CREATE || LOWORD(wparam) == WM_DESTROY)
            trace_add('P', ((DWORD)LOWORD(wparam) << 12) | (HIWORD(wparam) & 0xfffu));
        break;
    case WM_ORDER_POSTED: case WM_ORDER_SENT:
        trace_add('O', ((DWORD)(message - WM_APP) << 8) | (DWORD)(wparam & 0xff));
        break;
    }
}

static LRESULT CALLBACK child_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    int id = GetDlgCtrlID(hwnd);
    WNDPROC original = id == ID_BUTTON ? button_original : id == ID_EDIT ? edit_original : static_original;
    if (message == WM_DESTROY || message == WM_NCDESTROY)
        trace_add(id == ID_BUTTON ? 'B' : id == ID_EDIT ? 'E' : 'S', message);
    return CallWindowProcW(original, hwnd, message, wparam, lparam);
}

static HWND child(DWORD ex_style, const WCHAR *klass, const WCHAR *text, DWORD style, int x, int y, int w, int h,
                  HWND parent, int id, WNDPROC *original)
{
    HWND hwnd = CreateWindowExW(ex_style, klass, text, WS_CHILD | WS_VISIBLE | style, x, y, w, h, parent,
                                (HMENU)(UINT_PTR)id, instance, NULL);
    if (hwnd)
        *original = (WNDPROC)SetWindowLongPtrW(hwnd, GWLP_WNDPROC, (LONG_PTR)child_proc);
    return hwnd;
}

static LRESULT CALLBACK main_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    track_main(message, wparam);
    switch (message) {
    case WM_NCCREATE: {
        const CREATESTRUCTW *create = (const CREATESTRUCTW *)lparam;
        if (create->lpCreateParams == (LPVOID)&create_marker && create->hMenu == main_menu
            && create->hInstance == instance && create->x == 48 && create->y == 48 && create->cx == 480
            && create->cy == 320 && create->lpszName && weq(create->lpszName, title)
            && (create->style & WS_OVERLAPPEDWINDOW) == WS_OVERLAPPEDWINDOW && !(create->style & WS_VISIBLE))
            InterlockedOr(&create_checks, 1);
        break;
    }
    case WM_CREATE:
        if (((const CREATESTRUCTW *)lparam)->lpCreateParams != (LPVOID)&create_marker)
            return 0;
        button_hwnd = child(0, L"BUTTON", L"Run test", WS_TABSTOP | BS_PUSHBUTTON, 12, 12, 120, 28, hwnd, ID_BUTTON,
                            &button_original);
        static_hwnd = child(0, L"STATIC", L"Static \x03bb text", SS_LEFT, 144, 18, 200, 20, hwnd, ID_STATIC,
                            &static_original);
        edit_hwnd = child(WS_EX_CLIENTEDGE, L"EDIT", L"", WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
                          12, 48, 240, 96, hwnd, ID_EDIT, &edit_original);
        if (button_hwnd && static_hwnd && edit_hwnd && button_original && static_original && edit_original)
            InterlockedOr(&create_checks, 2);
        return 0;
    case WM_PAINT:
        paint_window(hwnd);
        return 0;
    case WM_COMMAND:
        if (LOWORD(wparam) == ID_EDIT && HIWORD(wparam) == EN_CHANGE && (HWND)lparam == edit_hwnd)
            InterlockedIncrement(&en_change);
        else if (LOWORD(wparam) == ID_BUTTON && HIWORD(wparam) == BN_CLICKED && (HWND)lparam == button_hwnd)
            InterlockedIncrement(&clicks);
        else if (LOWORD(wparam) == ID_FILE_RUN && HIWORD(wparam) == 0 && lparam == 0)
            InterlockedIncrement(&menu_commands);
        else if (LOWORD(wparam) == ID_FILE_RUN && HIWORD(wparam) == 1 && lparam == 0)
            InterlockedIncrement(&accelerator_commands);
        return 0;
    case WM_TIMER:
        if (wparam == 7 && InterlockedIncrement(&timer_ticks) == 3)
            KillTimer(hwnd, 7);
        return 0;
    case WM_SENTINEL:
        InterlockedIncrement(&sentinel_seen);
        return 0;
    case WM_ORDER_POSTED: case WM_ORDER_SENT:
        return 0;
    case WM_NESTED_OUTER: {
        LRESULT inner;
        nested_order[nested_steps++ & 3] = 1;
        nested_insend = InSendMessage() ? 1 : 0;
        inner = SendMessageW(hwnd, WM_NESTED_INNER, wparam + 1, 0);
        nested_order[nested_steps++ & 3] = 4;
        return inner * 10 + (LRESULT)wparam;
    }
    case WM_NESTED_INNER:
        nested_order[nested_steps++ & 3] = 2;
        if (!PostMessageW(hwnd, WM_NESTED_POSTED, 0, 0))
            fail(213);
        nested_order[nested_steps++ & 3] = 3;
        return (LRESULT)wparam * 2;
    case WM_NESTED_POSTED:
        InterlockedIncrement(&nested_posted);
        return 0;
    case WM_CROSS_CALLBACK:
        cross_order[InterlockedIncrement(&cross_steps) & 7] = 3;
        cross_insend_main = InSendMessage() ? 1 : 0;
        if (GetCurrentThreadId() == main_thread)
            InterlockedOr(&cross_main_ok, 1);
        return (LRESULT)wparam + 100;
    case WM_CROSS_POSTED:
        InterlockedIncrement(&cross_posted);
        return 0;
    case WM_DESTROY:
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static LRESULT CALLBACK worker_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == WM_CROSS_REQUEST) {
        LRESULT reply;
        cross_order[InterlockedIncrement(&cross_steps) & 7] = 2;
        cross_insend_worker = InSendMessage() ? 1 : 0;
        if (!PostMessageW(main_hwnd, WM_CROSS_POSTED, 0, 0))
            return 0;
        reply = SendMessageW(main_hwnd, WM_CROSS_CALLBACK, wparam, 0);
        cross_order[InterlockedIncrement(&cross_steps) & 7] = 4;
        return reply + 1000;
    }
    if (message == WM_DESTROY) {
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static DWORD WINAPI worker_main(LPVOID unused)
{
    MSG message;
    (void)unused;
    worker_thread = GetCurrentThreadId();
    worker_hwnd = CreateWindowExW(0, worker_class, L"worker", WS_POPUP, 0, 0, 16, 16, HWND_MESSAGE, NULL,
                                  instance, NULL);
    SetEvent(worker_ready);
    if (!worker_hwnd)
        return 1;
    while (GetMessageW(&message, NULL, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return 3;
}

static LRESULT CALLBACK hung_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == WM_HUNG_PROBE) {
        InterlockedIncrement(&hung_delivered);
        return 0x66;
    }
    if (message == WM_DESTROY) {
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

/* Creates a window, then blocks without retrieving messages until released. */
static DWORD WINAPI hung_main(LPVOID unused)
{
    MSG message;
    (void)unused;
    hung_hwnd = CreateWindowExW(0, hung_class, L"hung", WS_POPUP, 0, 0, 16, 16, HWND_MESSAGE, NULL, instance,
                                NULL);
    SetEvent(hung_ready);
    if (!hung_hwnd)
        return 1;
    if (WaitForSingleObject(hung_release, 30000) != WAIT_OBJECT_0)
        return 2;
    while (GetMessageW(&message, NULL, 0, 0) > 0)
        DispatchMessageW(&message);
    return 3;
}

static LRESULT CALLBACK reject_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    switch (message) {
    case WM_GETMINMAXINFO: case WM_NCCREATE: case WM_NCCALCSIZE: case WM_CREATE:
    case WM_DESTROY: case WM_NCDESTROY:
        trace_add('R', message);
        break;
    }
    if (message == WM_NCCREATE && reject_mode == 1)
        return FALSE;
    if (message == WM_CREATE && reject_mode == 2)
        return -1;
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

/* The message box is a #32770 dialog owned by the main window on this thread. */
static BOOL CALLBACK find_box(HWND hwnd, LPARAM found)
{
    WCHAR klass[16];
    if (GetClassNameW(hwnd, klass, 16) == 6 && weq(klass, L"#32770") && GetWindow(hwnd, GW_OWNER) == main_hwnd) {
        *(HWND *)found = hwnd;
        return FALSE;
    }
    return TRUE;
}

static void CALLBACK box_timer(HWND hwnd, UINT message, UINT_PTR id, DWORD time)
{
    HWND box = NULL;
    WCHAR text[64];
    (void)hwnd;
    (void)time;
    if (message != WM_TIMER)
        return;
    if (InterlockedIncrement(&box_attempts) > 100) {
        KillTimer(NULL, id);
        return;
    }
    EnumThreadWindows(GetCurrentThreadId(), find_box, (LPARAM)&box);
    if (!box || !IsWindowVisible(box))
        return;
    KillTimer(NULL, id);
    text[0] = 0;
    if (GetDlgItemTextW(box, 0xffff, text, 64) == wlen(box_text) && weq(text, box_text))
        InterlockedOr(&box_checks, 1);
    if (GetWindowTextW(box, text, 64) == 15 && weq(text, L"Doors self-test"))
        InterlockedOr(&box_checks, 2);
    PostMessageW(box, WM_COMMAND, MAKEWPARAM(IDCANCEL, BN_CLICKED), (LPARAM)GetDlgItem(box, IDCANCEL));
}

/* The hook types the full target path: the result is then independent of the
   initial folder the system chooses (Windows may prefer its per-application
   last-visited folder over lpstrInitialDir). */
static void hook_target(const WCHAR *directory, const WCHAR *name)
{
    DWORD length;
    hook_path[0] = 0;
    wappend(hook_path, MAX_PATH + 64, directory);
    length = wlen(hook_path);
    if (!length || hook_path[length - 1] != L'\\')
        wappend(hook_path, MAX_PATH + 64, L"\\");
    wappend(hook_path, MAX_PATH + 64, name);
}

static UINT_PTR CALLBACK file_hook(HWND page, UINT message, WPARAM wparam, LPARAM lparam)
{
    (void)wparam;
    if (message == WM_NOTIFY) {
        const OFNOTIFYW *note = (const OFNOTIFYW *)lparam;
        HWND dialog = GetParent(page);
        if (note->hdr.code == CDN_INITDONE) {
            InterlockedIncrement(&hook_initdone);
        } else if (note->hdr.code == CDN_FOLDERCHANGE && hook_initdone && hook_posts < 3) {
            /* The folder view is ready: name the file, then press the button.
               A press consumed by a pending navigation is repeated, bounded. */
            int button = hook_action == 2 ? IDCANCEL : IDOK;
            ++hook_posts;
            if (hook_action != 2)
                SendMessageW(dialog, CDM_SETCONTROLTEXT, GetDlgItem(dialog, cmb13) ? cmb13 : edt1,
                             (LPARAM)hook_path);
            PostMessageW(dialog, WM_COMMAND, MAKEWPARAM(button, BN_CLICKED), (LPARAM)GetDlgItem(dialog, button));
        } else if (note->hdr.code == CDN_FILEOK) {
            InterlockedIncrement(&hook_fileok);
        }
    }
    return 0;
}

/* ---- checks --------------------------------------------------------------------- */

static void register_classes(void)
{
    WNDCLASSEXW window;
    WNDCLASSEXW stored;
    zero(&window, sizeof(window));
    window.cbSize = sizeof(window);
    window.style = CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW;
    window.lpfnWndProc = main_proc;
    window.hInstance = instance;
    window.hCursor = LoadCursorW(NULL, IDC_ARROW);
    window.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    window.lpszClassName = main_class;
    if (!RegisterClassExW(&window))
        fail(100);
    SetLastError(SENTINEL);
    probe("register_duplicate_class", RegisterClassExW(&window) != 0, GetLastError());
    zero(&stored, sizeof(stored));
    stored.cbSize = sizeof(stored);
    if (!GetClassInfoExW(instance, main_class, &stored) || stored.style != window.style
        || stored.lpfnWndProc != main_proc || !(stored.style & CS_DBLCLKS))
        fail(101);
    window.style = 0;
    window.lpfnWndProc = worker_proc;
    window.hCursor = NULL;
    window.hbrBackground = NULL;
    window.lpszClassName = worker_class;
    if (!RegisterClassExW(&window))
        fail(102);
    window.lpfnWndProc = hung_proc;
    window.lpszClassName = hung_class;
    if (!RegisterClassExW(&window))
        fail(103);
    window.lpfnWndProc = reject_proc;
    window.lpszClassName = reject_class;
    if (!RegisterClassExW(&window))
        fail(104);
}

static void create_main(void)
{
    WCHAR text[64];
    trace_begin("create");
    main_menu = CreateMenu();
    file_menu = CreatePopupMenu();
    help_menu = CreatePopupMenu();
    if (!main_menu || !file_menu || !help_menu || !AppendMenuW(file_menu, MF_STRING, ID_FILE_RUN, L"&Run\tF5")
        || !AppendMenuW(file_menu, MF_SEPARATOR, 0, NULL) || !AppendMenuW(file_menu, MF_STRING, ID_FILE_EXIT, L"E&xit")
        || !AppendMenuW(help_menu, MF_STRING, ID_HELP_ABOUT, L"&About")
        || !AppendMenuW(main_menu, MF_POPUP, (UINT_PTR)file_menu, L"&File")
        || !AppendMenuW(main_menu, MF_POPUP, (UINT_PTR)help_menu, L"&Help"))
        fail(105);
    main_hwnd = CreateWindowExW(0, main_class, title, WS_OVERLAPPEDWINDOW, 48, 48, 480, 320, NULL, main_menu,
                                instance, (LPVOID)&create_marker);
    trace_end();
    if (!main_hwnd || create_checks != 3)
        fail(106);
    if (!(GetClassLongPtrW(main_hwnd, GCL_STYLE) & CS_DBLCLKS) || GetMenu(main_hwnd) != main_menu
        || IsWindowVisible(main_hwnd)
        || (GetWindowLongPtrW(main_hwnd, GWL_STYLE) & WS_OVERLAPPEDWINDOW) != WS_OVERLAPPEDWINDOW)
        fail(107);
    if ((DWORD)GetWindowTextW(main_hwnd, text, 64) != wlen(title) || !weq(text, title))
        fail(108);
    if (GetDlgItem(main_hwnd, ID_BUTTON) != button_hwnd || GetDlgItem(main_hwnd, ID_STATIC) != static_hwnd
        || GetDlgItem(main_hwnd, ID_EDIT) != edit_hwnd || GetParent(edit_hwnd) != main_hwnd
        || GetDlgCtrlID(edit_hwnd) != ID_EDIT)
        fail(109);
    if (GetClassNameW(button_hwnd, text, 64) != 6 || !weq(text, L"Button") || GetClassNameW(static_hwnd, text, 64) != 6
        || !weq(text, L"Static") || GetClassNameW(edit_hwnd, text, 64) != 4 || !weq(text, L"Edit"))
        fail(110);
    if (GetWindowTextW(static_hwnd, text, 64) != 13 || !weq(text, L"Static \x03bb text"))
        fail(111);
    compare_trace(0, expected_create, sizeof(expected_create) / sizeof(DWORD), 120);
}

static void show_and_paint(void)
{
    RECT client, window, update, part;
    POINT point;
    LONG before;
    trace_begin("show");
    ShowWindow(main_hwnd, SW_SHOWNOACTIVATE);
    UpdateWindow(main_hwnd);
    trace_end();
    compare_trace(1, expected_show, sizeof(expected_show) / sizeof(DWORD), 121);
    if (!IsWindowVisible(main_hwnd) || paint_count != 1)
        fail(152);
    if (!GetClientRect(main_hwnd, &client) || !GetWindowRect(main_hwnd, &window) || client.left || client.top
        || client.right <= 0 || client.bottom <= 0 || window.right - window.left != 480
        || window.bottom - window.top != 320 || client.right >= 480 || client.bottom >= 320)
        fail(153);
    point.x = 5;
    point.y = 7;
    if (!ClientToScreen(main_hwnd, &point) || point.x <= window.left || point.y <= window.top
        || !ScreenToClient(main_hwnd, &point) || point.x != 5 || point.y != 7)
        fail(154);
    if (GetUpdateRect(main_hwnd, &update, FALSE))
        fail(155);
    before = paint_count;
    SetRect(&part, 20, 30, 60, 70);
    if (!InvalidateRect(main_hwnd, &part, FALSE) || !GetUpdateRect(main_hwnd, &update, FALSE)
        || update.left > 20 || update.top > 30 || update.right < 60 || update.bottom < 70)
        fail(156);
    if (!ValidateRect(main_hwnd, NULL) || GetUpdateRect(main_hwnd, &update, FALSE) || !UpdateWindow(main_hwnd)
        || paint_count != before)
        fail(157);
    InvalidateRect(main_hwnd, NULL, TRUE);
    UpdateWindow(main_hwnd);
    if (paint_count != before + 1 || GetUpdateRect(main_hwnd, &update, FALSE))
        fail(158);
}

static void raster(void)
{
    DWORD gdi_before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
    DWORD second;
    raster_crc = render_raster(TRUE);
    second = render_raster(FALSE);
    if (raster_crc != second)
        fail(159);
    if (GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) != gdi_before)
        fail(160);
}

/* Sent messages run synchronously; posted ones are retrieved in order, and a
   pending WM_PAINT is generated only once no posted message remains. */
static void message_order(void)
{
    trace_begin("order");
    InvalidateRect(main_hwnd, NULL, FALSE);
    if (!PostMessageW(main_hwnd, WM_ORDER_POSTED, 1, 0) || !PostMessageW(main_hwnd, WM_ORDER_POSTED, 2, 0))
        fail(122);
    SendMessageW(main_hwnd, WM_ORDER_SENT, 3, 0);
    if (tracing->count != 1 || tracing->entries[0] != T('O', 0xb03))
        fail(123);
    while (pump_once(0))
        ;
    trace_end();
    compare_trace(2, expected_order, sizeof(expected_order) / sizeof(DWORD), 124);
}

static void post_key(HWND target, UINT key, BOOL extended)
{
    UINT scan = MapVirtualKeyW(key, MAPVK_VK_TO_VSC);
    LPARAM bits = 1 | ((LPARAM)scan << 16) | (extended ? ((LPARAM)1 << 24) : 0);
    if (!PostMessageW(target, WM_KEYDOWN, key, bits))
        fail(161);
}

static void post_chars(HWND target, const WCHAR *text)
{
    while (*text)
        if (!PostMessageW(target, WM_CHAR, *text++, 1))
            fail(162);
}

static void input(void)
{
    WCHAR text[64];
    DWORD start = 0, end = 0;
    post_chars(edit_hwnd, L"Doors\r\x03bb\x9580");
    post_key(edit_hwnd, VK_LEFT, TRUE);
    post_chars(edit_hwnd, L"x");
    post_key(edit_hwnd, VK_HOME, TRUE);
    post_key(edit_hwnd, VK_DELETE, TRUE);
    post_key(edit_hwnd, VK_END, TRUE);
    post_chars(edit_hwnd, L"\x0008\x00e9");
    drain(163);
    if (SendMessageW(edit_hwnd, WM_GETTEXTLENGTH, 0, 0) != 9
        || SendMessageW(edit_hwnd, WM_GETTEXT, 64, (LPARAM)text) != 9 || !weq(text, stage_one))
        fail(164);
    if (SendMessageW(edit_hwnd, EM_GETLINECOUNT, 0, 0) != 2 || SendMessageW(edit_hwnd, EM_LINEINDEX, 1, 0) != 7)
        fail(165);
    SendMessageW(edit_hwnd, EM_SETSEL, 7, 8);
    SendMessageW(edit_hwnd, EM_GETSEL, (WPARAM)&start, (LPARAM)&end);
    if (start != 7 || end != 8)
        fail(166);
    post_chars(edit_hwnd, L"Z");
    drain(167);
    if (SendMessageW(edit_hwnd, WM_GETTEXT, 64, (LPARAM)text) != 9 || !weq(text, stage_two)
        || !SendMessageW(edit_hwnd, EM_GETMODIFY, 0, 0))
        fail(168);
    probe("edit_en_change", (DWORD)en_change, 0);
    if (!SetWindowTextW(static_hwnd, L"Static \x9580") || GetWindowTextW(static_hwnd, text, 64) != 8
        || !weq(text, L"Static \x9580"))
        fail(169);
    SendMessageW(button_hwnd, BM_CLICK, 0, 0);
    drain(170);
    if (clicks != 1)
        fail(171);
}

static void menu(void)
{
    WCHAR text[32];
    ACCEL entry;
    HACCEL table;
    MSG key;
    if (GetMenuItemCount(main_menu) != 2 || GetMenuStringW(main_menu, 0, text, 32, MF_BYPOSITION) != 5
        || !weq(text, L"&File") || GetSubMenu(main_menu, 0) != file_menu || GetMenuItemCount(file_menu) != 3
        || GetMenuItemID(file_menu, 0) != ID_FILE_RUN || GetMenuItemID(file_menu, 1) != 0)
        fail(180);
    if (GetMenuStringW(file_menu, ID_FILE_RUN, text, 32, MF_BYCOMMAND) != 7 || !weq(text, L"&Run\tF5"))
        fail(181);
    if (CheckMenuItem(file_menu, ID_FILE_RUN, MF_BYCOMMAND | MF_CHECKED) != MF_UNCHECKED
        || !(GetMenuState(file_menu, ID_FILE_RUN, MF_BYCOMMAND) & MF_CHECKED)
        || EnableMenuItem(file_menu, ID_FILE_EXIT, MF_BYCOMMAND | MF_GRAYED) != MF_ENABLED
        || !(GetMenuState(file_menu, ID_FILE_EXIT, MF_BYCOMMAND) & MF_GRAYED) || !DrawMenuBar(main_hwnd))
        fail(182);
    if (!PostMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_FILE_RUN, 0), 0))
        fail(183);
    drain(184);
    if (menu_commands != 1)
        fail(185);
    entry.fVirt = FVIRTKEY;
    entry.key = VK_F5;
    entry.cmd = ID_FILE_RUN;
    table = CreateAcceleratorTableW(&entry, 1);
    zero(&key, sizeof(key));
    key.hwnd = main_hwnd;
    key.message = WM_KEYDOWN;
    key.wParam = VK_F5;
    key.lParam = 1 | ((LPARAM)MapVirtualKeyW(VK_F5, MAPVK_VK_TO_VSC) << 16);
    if (!table || !TranslateAcceleratorW(main_hwnd, table, &key) || accelerator_commands != 1
        || !DestroyAcceleratorTable(table))
        fail(186);
}

static void timer(void)
{
    if (SetTimer(main_hwnd, 7, 10, NULL) != 7)
        fail(190);
    pump_until(&timer_ticks, 3, 3000, 191);
    pump_for(80);
    probe("timer_count", (DWORD)timer_ticks, 0);
}

static void message_box(void)
{
    int result;
    if (!SetTimer(NULL, 0, 20, box_timer))
        fail(200);
    SetLastError(0);
    result = MessageBoxW(main_hwnd, box_text, L"Doors self-test", MB_OKCANCEL | MB_ICONINFORMATION);
    probe("msgbox_result", (DWORD)result, GetLastError());
    if (box_checks != 3)
        fail(201);
}

static void reentrancy(void)
{
    HANDLE thread;
    DWORD id, code;
    LRESULT result;
    nested_steps = 0;
    result = SendMessageW(main_hwnd, WM_NESTED_OUTER, 5, 0);
    if (result != 125 || nested_steps != 4 || nested_order[0] != 1 || nested_order[1] != 2 || nested_order[2] != 3
        || nested_order[3] != 4 || nested_posted != 0)
        fail(210);
    probe("nested_insend", (DWORD)nested_insend, 0);
    drain(211);
    if (nested_posted != 1)
        fail(212);

    worker_ready = CreateEventW(NULL, TRUE, FALSE, NULL);
    thread = worker_ready ? CreateThread(NULL, 0, worker_main, NULL, 0, &id) : NULL;
    if (!thread || WaitForSingleObject(worker_ready, 5000) != WAIT_OBJECT_0 || !worker_hwnd)
        fail(214);
    cross_order[InterlockedIncrement(&cross_steps) & 7] = 1;
    result = SendMessageW(worker_hwnd, WM_CROSS_REQUEST, 7, 0);
    cross_order[InterlockedIncrement(&cross_steps) & 7] = 5;
    if (result != 1107 || cross_steps != 5 || cross_order[1] != 1 || cross_order[2] != 2 || cross_order[3] != 3
        || cross_order[4] != 4 || cross_order[5] != 5 || cross_main_ok != 1 || cross_posted != 0)
        fail(215);
    probe("cross_thread_insend", (DWORD)cross_insend_worker, (DWORD)cross_insend_main);
    drain(216);
    if (cross_posted != 1)
        fail(217);
    SetLastError(SENTINEL);
    probe("destroy_other_thread", DestroyWindow(worker_hwnd) != 0, GetLastError());
    if (!IsWindow(worker_hwnd) || !PostMessageW(worker_hwnd, WM_CLOSE, 0, 0)
        || WaitForSingleObject(thread, 5000) != WAIT_OBJECT_0 || !GetExitCodeThread(thread, &code) || code != 3
        || IsWindow(worker_hwnd))
        fail(218);
    CloseHandle(thread);
    CloseHandle(worker_ready);
}

static void destroyed(void)
{
    WCHAR text[8];
    MSG message;
    DWORD_PTR result = 0;
    HWND dead = CreateWindowExW(0, main_class, L"dead", WS_POPUP, 0, 0, 10, 10, NULL, NULL, instance, NULL);
    if (!dead || !DestroyWindow(dead))
        fail(230);
    SetLastError(SENTINEL);
    probe("iswindow_destroyed", IsWindow(dead) != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("send_destroyed", (DWORD)SendMessageW(dead, WM_USER, 0, 0), GetLastError());
    SetLastError(SENTINEL);
    probe("post_destroyed", PostMessageW(dead, WM_USER, 0, 0) != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("destroy_destroyed", DestroyWindow(dead) != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("gettext_destroyed", (DWORD)GetWindowTextW(dead, text, 8), GetLastError());
    SetLastError(SENTINEL);
    probe("settext_destroyed", SetWindowTextW(dead, L"x") != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("peek_destroyed", PeekMessageW(&message, dead, 0, 0, PM_NOREMOVE) != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("send_timeout_destroyed", SendMessageTimeoutW(dead, WM_USER, 0, 0, SMTO_NORMAL, 100, &result) != 0,
          GetLastError());
    if (IsWindow(dead))
        fail(231);
}

static void rejected(void)
{
    HWND hwnd;
    SetLastError(SENTINEL);
    hwnd = CreateWindowExW(0, missing_class, L"", WS_POPUP, 0, 0, 10, 10, NULL, NULL, instance, NULL);
    probe("create_unknown_class", hwnd != NULL, GetLastError());
    SetLastError(SENTINEL);
    hwnd = CreateWindowExW(0, reject_class, L"", WS_CHILD, 0, 0, 10, 10, NULL, NULL, instance, NULL);
    probe("create_child_without_parent", hwnd != NULL, GetLastError());
    {
        HWND dead = CreateWindowExW(0, main_class, L"dead", WS_POPUP, 0, 0, 10, 10, NULL, NULL, instance, NULL);
        if (!dead || !DestroyWindow(dead))
            fail(240);
        SetLastError(SENTINEL);
        hwnd = CreateWindowExW(0, reject_class, L"", WS_CHILD, 0, 0, 10, 10, dead, NULL, instance, NULL);
        probe("create_invalid_parent", hwnd != NULL, GetLastError());
    }
    reject_mode = 1;
    trace_begin("reject_nc");
    SetLastError(SENTINEL);
    hwnd = CreateWindowExW(0, reject_class, L"nc", WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, NULL, NULL, instance, NULL);
    probe("create_nccreate_false", hwnd != NULL, GetLastError());
    trace_end();
    reject_mode = 2;
    trace_begin("reject_create");
    SetLastError(SENTINEL);
    hwnd = CreateWindowExW(0, reject_class, L"create", WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, NULL, NULL, instance,
                           NULL);
    probe("create_create_minus_one", hwnd != NULL, GetLastError());
    trace_end();
    reject_mode = 0;
    compare_trace(3, expected_reject_nc, sizeof(expected_reject_nc) / sizeof(DWORD), 125);
    compare_trace(4, expected_reject_create, sizeof(expected_reject_create) / sizeof(DWORD), 126);
    SetLastError(SENTINEL);
    probe("unregister_class_in_use", UnregisterClassW(main_class, instance) != 0, GetLastError());
    SetLastError(SENTINEL);
    probe("unregister_missing_class", UnregisterClassW(missing_class, instance) != 0, GetLastError());
}

static void hung(void)
{
    HANDLE thread;
    DWORD id, code;
    DWORD_PTR result = 0;
    ULONGLONG started, elapsed;
    hung_ready = CreateEventW(NULL, TRUE, FALSE, NULL);
    hung_release = CreateEventW(NULL, TRUE, FALSE, NULL);
    thread = hung_ready && hung_release ? CreateThread(NULL, 0, hung_main, NULL, 0, &id) : NULL;
    if (!thread || WaitForSingleObject(hung_ready, 5000) != WAIT_OBJECT_0 || !hung_hwnd)
        fail(250);
    SetLastError(SENTINEL);
    probe("is_hung_initial", IsHungAppWindow(hung_hwnd) != 0, GetLastError());
    started = GetTickCount64();
    SetLastError(SENTINEL);
    probe("send_timeout_busy", SendMessageTimeoutW(hung_hwnd, WM_HUNG_PROBE, 0, 0, SMTO_BLOCK, 200, &result) != 0,
          GetLastError());
    elapsed = GetTickCount64() - started;
    probe("send_timeout_busy_waited", elapsed >= 150 && elapsed < 2000, 0);
    /* The hung threshold is five seconds without retrieving messages. */
    pump_for(5600);
    SetLastError(SENTINEL);
    probe("is_hung_after", IsHungAppWindow(hung_hwnd) != 0, GetLastError());
    started = GetTickCount64();
    SetLastError(SENTINEL);
    probe("send_timeout_hung_abort",
          SendMessageTimeoutW(hung_hwnd, WM_HUNG_PROBE, 0, 0, SMTO_ABORTIFHUNG, 1000, &result) != 0, GetLastError());
    elapsed = GetTickCount64() - started;
    probe("send_timeout_hung_fast", elapsed < 500, 0);
    if (!SetEvent(hung_release))
        fail(251);
    /* Sent messages are processed in order: this reply follows any late ones. */
    if (!SendMessageTimeoutW(hung_hwnd, WM_HUNG_PROBE, 1, 0, SMTO_BLOCK, 5000, &result) || result != 0x66)
        fail(252);
    probe("hung_late_delivery", (DWORD)hung_delivered - 1, 0);
    if (!PostMessageW(hung_hwnd, WM_CLOSE, 0, 0) || WaitForSingleObject(thread, 5000) != WAIT_OBJECT_0
        || !GetExitCodeThread(thread, &code) || code != 3)
        fail(253);
    CloseHandle(thread);
    CloseHandle(hung_ready);
    CloseHandle(hung_release);
}

static void current_directory(WCHAR *buffer, DWORD capacity)
{
    DWORD count = GetCurrentDirectoryW(capacity, buffer);
    if (!count || count >= capacity - 64)
        fail(270);
}

static void prepare_dialog(OPENFILENAMEW *dialog, WCHAR *path, DWORD capacity, const WCHAR *directory, DWORD flags)
{
    zero(dialog, sizeof(*dialog));
    dialog->lStructSize = sizeof(*dialog);
    dialog->hwndOwner = main_hwnd;
    dialog->hInstance = instance;
    dialog->lpstrFilter = L"Text documents (*.txt)\0*.txt\0All files (*.*)\0*.*\0";
    dialog->nFilterIndex = 1;
    dialog->lpstrFile = path;
    dialog->nMaxFile = capacity;
    dialog->lpstrInitialDir = directory;
    dialog->lpstrDefExt = L"txt";
    dialog->lpfnHook = file_hook;
    dialog->Flags = OFN_EXPLORER | OFN_ENABLEHOOK | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT | flags;
}

/* The returned path must name file_name inside the initial directory. */
static void check_path(const OPENFILENAMEW *dialog, const WCHAR *directory, UINT code)
{
    WCHAR expected[MAX_PATH + 64];
    DWORD length;
    expected[0] = 0;
    wappend(expected, MAX_PATH + 64, directory);
    length = wlen(expected);
    if (!length || expected[length - 1] != L'\\')
        wappend(expected, MAX_PATH + 64, L"\\");
    length = wlen(expected);
    wappend(expected, MAX_PATH + 64, file_name);
    if (!weq_ascii_nocase(dialog->lpstrFile, expected) || dialog->nFileOffset != length
        || dialog->nFileExtension != length + wlen(file_name) - 3)
        fail(code);
}

static void dialogs(void)
{
    static WCHAR directory[MAX_PATH + 64];
    WCHAR *path = saved_path;
    static WCHAR tiny[8];
    OPENFILENAMEW dialog;
    BOOL ok;
    current_directory(directory, MAX_PATH + 64);

    prepare_dialog(&dialog, path, MAX_PATH + 64, directory, 0);
    dialog.lStructSize = 0;
    SetLastError(SENTINEL);
    ok = GetSaveFileNameW(&dialog);
    probe("save_struct_size", ok != 0, CommDlgExtendedError());
    stage("dialog-cancel");

    path[0] = 0;
    prepare_dialog(&dialog, path, MAX_PATH + 64, directory, 0);
    hook_action = 2;
    hook_initdone = 0;
    hook_posts = 0;
    ok = GetSaveFileNameW(&dialog);
    probe("save_cancel", ok != 0, CommDlgExtendedError());
    stage("dialog-small");
    if (hook_initdone != 1)
        fail(271);

    tiny[0] = 0;
    prepare_dialog(&dialog, tiny, 8, directory, 0);
    hook_target(directory, long_name);
    hook_action = 3;
    hook_initdone = 0;
    hook_posts = 0;
    ok = GetSaveFileNameW(&dialog);
    probe("save_buffer_small", ok != 0, CommDlgExtendedError());
    stage("dialog-save");
    probe("save_buffer_small_reported", hook_initdone == 1 && *(const WORD *)tiny > 8, 0);

    path[0] = 0;
    wappend(path, MAX_PATH + 64, file_name);
    prepare_dialog(&dialog, path, MAX_PATH + 64, directory, 0);
    hook_target(directory, file_name);
    hook_action = 1;
    hook_initdone = 0;
    hook_posts = 0;
    hook_fileok = 0;
    ok = GetSaveFileNameW(&dialog);
    probe("save_ok", ok != 0, CommDlgExtendedError());
    if (!ok || hook_initdone != 1 || hook_fileok != 1)
        fail(272);
    check_path(&dialog, directory, 273);
}

static void write_file(const WCHAR *path)
{
    static BYTE buffer[512];
    DWORD bytes = wlen(file_text) * 2, actual = 0, i;
    const BYTE *expected = (const BYTE *)file_text;
    HANDLE file = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE)
        fail(290);
    if (!WriteFile(file, file_text, bytes, &actual, NULL) || actual != bytes || !FlushFileBuffers(file))
        fail(291);
    if (SetFilePointer(file, 0, NULL, FILE_BEGIN) != 0 || !ReadFile(file, buffer, sizeof(buffer), &actual, NULL)
        || actual != bytes)
        fail(292);
    for (i = 0; i < bytes; ++i)
        if (buffer[i] != expected[i])
            fail(293);
    if (!CloseHandle(file))
        fail(294);
}

static void destroy_main(void)
{
    trace_begin("destroy");
    if (!DestroyWindow(main_hwnd))
        fail(127);
    trace_end();
    compare_trace(5, expected_destroy, sizeof(expected_destroy) / sizeof(DWORD), 128);
    if (IsWindow(main_hwnd) || IsWindow(edit_hwnd) || IsMenu(main_menu))
        fail(129);
    if (!UnregisterClassW(main_class, instance) || !UnregisterClassW(worker_class, instance)
        || !UnregisterClassW(hung_class, instance) || !UnregisterClassW(reject_class, instance))
        fail(130);
}

static void check_probes(void)
{
    DWORD i;
    const char *a, *b;
    if (probing)
        return;
    if (probe_count != sizeof(expected_probes) / sizeof(expected_probes[0]))
        fail(320);
    for (i = 0; i < probe_count; ++i) {
        a = probes[i].name;
        b = expected_probes[i].name;
        while (*a && *a == *b) {
            ++a;
            ++b;
        }
        if (*a != *b || probes[i].ret != expected_probes[i].ret || probes[i].error != expected_probes[i].error) {
            line_len = 0;
            put("WINDOWS_GUI_FAIL code=");
            put_dec(321 + i);
            put(" probe=");
            put(probes[i].name);
            put(" ret=");
            put_dec(probes[i].ret);
            put(" error=");
            put_dec(probes[i].error);
            put("\r\n");
            flush_line();
            ExitProcess(321 + i);
        }
    }
}

static void print_probes(void)
{
    DWORD i, j;
    HANDLE file;
    DWORD actual = 0;
    for (i = 0; i < probe_count; ++i) {
        put("WINDOWS_GUI_PROBE name=");
        put(probes[i].name);
        put(" ret=");
        put_dec(probes[i].ret);
        put(" error=");
        put_dec(probes[i].error);
        put("\r\n");
        flush_line();
    }
    for (i = 0; i < trace_count; ++i) {
        put("WINDOWS_GUI_TRACE phase=");
        put(traces[i].name);
        put(" entries=");
        for (j = 0; j < traces[i].count; ++j) {
            if (j)
                put_char(',');
            put_char((char)(traces[i].entries[j] >> 24));
            put_char(':');
            put_hex(traces[i].entries[j] & 0xffffffu, 4);
        }
        put("\r\n");
        flush_line();
    }
    file = CreateFileW(L"wingui-raster.bin", GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE || !WriteFile(file, raster_pixels, sizeof(raster_pixels), &actual, NULL)
        || actual != sizeof(raster_pixels) || !CloseHandle(file))
        fail(310);
    put("WINDOWS_GUI_PROBE raster crc=");
    put_hex(raster_crc, 8);
    put(" bytes=");
    put_dec(sizeof(raster_pixels));
    put("\r\n");
    flush_line();
}

static void reopen(void)
{
    static WCHAR directory[MAX_PATH + 64];
    static WCHAR path[MAX_PATH + 64];
    static BYTE buffer[512];
    OPENFILENAMEW dialog;
    DWORD bytes = wlen(file_text) * 2, actual = 0, i;
    const BYTE *expected = (const BYTE *)file_text;
    HANDLE file;
    main_hwnd = CreateWindowExW(0, main_class, title, WS_OVERLAPPEDWINDOW, 48, 48, 480, 320, NULL, NULL, instance,
                                NULL);
    if (!main_hwnd)
        fail(300);
    current_directory(directory, MAX_PATH + 64);
    path[0] = 0;
    wappend(path, MAX_PATH + 64, file_name);
    hook_target(directory, file_name);
    prepare_dialog(&dialog, path, MAX_PATH + 64, directory, OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST);
    hook_action = 1;
    if (!GetOpenFileNameW(&dialog) || hook_initdone != 1 || hook_fileok != 1)
        fail(301);
    check_path(&dialog, directory, 302);
    file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE || GetFileSize(file, NULL) != bytes)
        fail(303);
    if (!ReadFile(file, buffer, sizeof(buffer), &actual, NULL) || actual != bytes || !CloseHandle(file))
        fail(304);
    for (i = 0; i < bytes; ++i)
        if (buffer[i] != expected[i])
            fail(305);
    if (!DestroyWindow(main_hwnd))
        fail(306);
    put("WINDOWS_GUI_REOPEN dialog=1 file=1 bytes=");
    put_dec(bytes);
    put("\r\n");
    flush_line();
    ExitProcess(EXIT_REOPEN);
}

/* The single argument after the program name, or an empty string. */
static const WCHAR *argument(void)
{
    const WCHAR *text = GetCommandLineW();
    if (*text == L'"') {
        ++text;
        while (*text && *text != L'"')
            ++text;
        if (*text)
            ++text;
    } else {
        while (*text && *text != L' ' && *text != L'\t')
            ++text;
    }
    while (*text == L' ' || *text == L'\t')
        ++text;
    return text;
}

__declspec(noreturn) void entry(void)
{
    const WCHAR *mode = argument();
    instance = GetModuleHandleW(NULL);
    main_thread = GetCurrentThreadId();
    out_handle = GetStdHandle(STD_OUTPUT_HANDLE);
    if (!instance)
        fail(92);
    if (weq(mode, L"--probe"))
        probing = TRUE;
    else if (mode[0] && !weq(mode, L"--reopen"))
        fail(93);
    results_handle = CreateFileW(probing ? L"wingui-selftest-probe.results"
                                 : mode[0] ? L"wingui-selftest-reopen.results" : L"wingui-selftest.results",
                                 GENERIC_WRITE, FILE_SHARE_READ, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (results_handle == INVALID_HANDLE_VALUE)
        fail(94);
    register_classes();
    if (weq(mode, L"--reopen"))
        reopen();
    stage("create");
    create_main();
    stage("show");
    show_and_paint();
    stage("raster");
    raster();
    stage("order");
    message_order();
    stage("input");
    input();
    stage("menu");
    menu();
    stage("timer");
    timer();
    stage("msgbox");
    message_box();
    stage("reentrant");
    reentrancy();
    stage("destroyed");
    destroyed();
    stage("rejected");
    rejected();
    stage("hung");
    hung();
    stage("dialog");
    dialogs();
    stage("file");
    write_file(saved_path);
    stage("destroy");
    destroy_main();
    check_probes();
    if (probing) {
        print_probes();
        ExitProcess(EXIT_PROBE);
    }
    put("WINDOWS_GUI_OK create=1 order=1 paint_crc=");
    put_hex(raster_crc, 8);
    put(" input=1 menu=1 timer=1 msgbox=1 reentrant=1 destroyed=1 rejected=1 hung=1 dialog=1 file=1\r\n");
    flush_line();
    ExitProcess(EXIT_OK);
}

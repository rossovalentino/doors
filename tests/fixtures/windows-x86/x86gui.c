/* X86-B probe (P17 M9): a 32-bit (x86) Windows program using user32 and
 * gdi32, MSVC, no C runtime:
 *
 *   x86gui.exe raw | check
 *
 * It creates hidden windows only (nothing appears on screen, nothing is
 * changed) and checks what a 32-bit program sees of the structures and
 * callbacks whose layout differs from x64: WNDCLASSEXW and GetClassInfoExW,
 * CREATESTRUCTW in WM_NCCREATE/WM_CREATE, window and class longs (4-byte
 * extra-byte slots, GWL_WNDPROC), MSG from PeekMessageW, LRESULT and
 * LPARAM sign, cross-thread SendMessageW (WM_SETTEXT, WM_GETTEXT and
 * SetWindowPos's WINDOWPOS, marshalled), SendMessageTimeoutW's DWORD_PTR,
 * PAINTSTRUCT, CreateDIBSection's bits pointer, BITMAP and LOGBRUSH from
 * GetObjectW, a menu's item data, a modeless dialog's DWLP_USER and
 * DWLP_MSGRESULT, a TIMERPROC and EnumChildWindows. Every structure the
 * system fills is followed by a canary that must survive. `raw` prints
 * the lines (x86gui.expected.h is generated from Windows 11's output);
 * `check` compares. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>

#pragma function(memset, memcpy)
void *memset(void *d, int v, size_t n) {
    unsigned char *p = (unsigned char *)d;
    while (n--) *p++ = (unsigned char)v;
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = (unsigned char *)d;
    const unsigned char *q = (const unsigned char *)s;
    while (n--) *p++ = *q++;
    return d;
}

#define CANARY 0x5a5aa5a5ul

static HANDLE out;
static char captured[8192];
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
static void num(long v) {
    char b[16];
    int i = 15, neg = v < 0;
    unsigned long u = neg ? (unsigned long)-v : (unsigned long)v;
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
static void nl(void) { put("\r\n"); }
static void field(const char *name, long value) {
    put(" ");
    put(name);
    put("=");
    num(value);
}
static void wput(const wchar_t *s) {
    char b[2];
    b[1] = 0;
    while (*s) {
        b[0] = *s < 128 ? (char)*s : '?';
        put(b);
        s++;
    }
}
static int wsame(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}

static const wchar_t CLASS_NAME[] = L"DoorsX86Probe";
static HINSTANCE instance;
static HWND window;
static int cookie = 7;
/* What the window procedure saw. */
static UINT sequence[16];
static int sequence_length, recording;
static int nc_params, nc_instance, nc_parent, nc_name, nc_class, create_params, create_name, create_style, create_ex;
static int create_cx, create_cy, create_x, create_y;
static int posing_x, posing_y, posing_cx, posing_cy, posing_seen;

static LRESULT CALLBACK window_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
    if (recording && sequence_length < 16) sequence[sequence_length++] = message;
    switch (message) {
    case WM_NCCREATE: {
        const CREATESTRUCTW *c = (const CREATESTRUCTW *)lparam;
        nc_params = c->lpCreateParams == &cookie;
        nc_instance = c->hInstance == instance;
        nc_parent = c->hwndParent == NULL;
        nc_name = c->lpszName != NULL && wsame(c->lpszName, L"x86 window");
        /* 1: an atom, 2: the class name as a copy, 3: the caller's pointer. */
        nc_class = !HIWORD((DWORD_PTR)c->lpszClass) ? 1 : c->lpszClass == CLASS_NAME ? 3 : wsame(c->lpszClass, CLASS_NAME) ? 2 : 4;
        break;
    }
    case WM_CREATE: {
        const CREATESTRUCTW *c = (const CREATESTRUCTW *)lparam;
        create_params = c->lpCreateParams == &cookie;
        create_name = c->lpszName != NULL && wsame(c->lpszName, L"x86 window");
        create_style = (c->style & WS_OVERLAPPEDWINDOW) == WS_OVERLAPPEDWINDOW;
        create_ex = (int)c->dwExStyle;
        create_cx = c->cx;
        create_cy = c->cy;
        create_x = c->x;
        create_y = c->y;
        return 0;
    }
    case WM_WINDOWPOSCHANGING: {
        const WINDOWPOS *p = (const WINDOWPOS *)lparam;
        if (p->hwnd == hwnd && !(p->flags & SWP_NOMOVE) && !(p->flags & SWP_NOSIZE)) {
            posing_x = p->x;
            posing_y = p->y;
            posing_cx = p->cx;
            posing_cy = p->cy;
            posing_seen = 1;
        }
        break;
    }
    case WM_APP + 1:
        return (LRESULT)wparam + lparam;
    case WM_APP + 2:
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static void class_step(void) {
    WNDCLASSEXW wc;
    struct {
        WNDCLASSEXW info;
        DWORD canary;
    } got;
    ATOM atom;
    memset(&wc, 0, sizeof wc);
    wc.cbSize = sizeof wc;
    wc.lpfnWndProc = window_proc;
    wc.cbClsExtra = 8;
    wc.cbWndExtra = 16;
    wc.hInstance = instance;
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = CLASS_NAME;
    atom = RegisterClassExW(&wc);
    memset(&got, 0, sizeof got);
    got.info.cbSize = sizeof got.info;
    got.canary = CANARY;
    put("class size=");
    num((long)sizeof wc);
    field("registered", atom != 0);
    field("info", GetClassInfoExW(instance, CLASS_NAME, &got.info) == atom);
    field("proc", got.info.lpfnWndProc == window_proc);
    field("extra", got.info.cbWndExtra);
    field("class-extra", got.info.cbClsExtra);
    field("name", got.info.lpszClassName == CLASS_NAME);
    field("canary", got.canary == CANARY);
    nl();
}

static void create_step(void) {
    LONG proc;
    recording = 1;
    window = CreateWindowExW(0, CLASS_NAME, L"x86 window", WS_OVERLAPPEDWINDOW, 10, 20, 300, 200, NULL, NULL, instance,
                             &cookie);
    recording = 0;
    put("create window=");
    num(window != NULL);
    put(" sequence=");
    {
        int i;
        for (i = 0; i < sequence_length && i < 4; i++) {
            if (i) put(",");
            hex(sequence[i]);
        }
    }
    nl();
    put("createstruct params=");
    num(nc_params);
    field("instance", nc_instance);
    field("parent", nc_parent);
    field("name", nc_name);
    field("class", nc_class);
    field("create-params", create_params);
    field("create-name", create_name);
    field("style", create_style);
    put(" ex=");
    hex((unsigned long)create_ex);
    put(" size=");
    num(create_cx);
    put("x");
    num(create_cy);
    put(" at=");
    num(create_x);
    put(",");
    num(create_y);
    nl();
    proc = GetWindowLongW(window, GWL_WNDPROC);
    put("longs wndproc=");
    num(proc == (LONG)window_proc);
    field("class-wndproc", GetClassLongW(window, GCL_WNDPROC) == (DWORD)window_proc);
    field("class-extra", (long)GetClassLongW(window, GCL_CBWNDEXTRA));
    SetWindowLongW(window, 0, 0x11111111);
    SetWindowLongW(window, 4, 0x22222222);
    SetWindowLongW(window, 12, 0x44444444);
    SetWindowLongW(window, GWL_USERDATA, (LONG)&cookie);
    field("extra0", GetWindowLongW(window, 0) == 0x11111111);
    field("extra4", GetWindowLongW(window, 4) == 0x22222222);
    field("extra12", GetWindowLongW(window, 12) == 0x44444444);
    SetLastError(0);
    field("extra13", GetWindowLongW(window, 13));
    field("error", (long)GetLastError());
    field("userdata", GetWindowLongW(window, GWL_USERDATA) == (LONG)&cookie);
    nl();
}

static void message_step(void) {
    struct {
        MSG msg;
        DWORD canary;
    } m;
    struct {
        DWORD_PTR result;
        DWORD canary;
    } r;
    LRESULT sent = SendMessageW(window, WM_APP + 1, (WPARAM)0xfffffffful, -2);
    put("send result=");
    num((long)sent);
    PostMessageW(window, WM_APP + 2, 7, -5);
    m.canary = CANARY;
    field("peek", PeekMessageW(&m.msg, window, WM_APP + 2, WM_APP + 2, PM_REMOVE));
    field("hwnd", m.msg.hwnd == window);
    field("wparam", (long)m.msg.wParam);
    field("lparam", (long)m.msg.lParam);
    field("canary", m.canary == CANARY);
    DispatchMessageW(&m.msg);
    r.result = 0;
    r.canary = CANARY;
    field("timeout", (long)SendMessageTimeoutW(window, WM_APP + 1, 1, 2, SMTO_NORMAL, 1000, &r.result) != 0);
    field("timeout-result", (long)r.result);
    field("timeout-canary", r.canary == CANARY);
    nl();
}

/* Cross-thread sends: the window's thread pumps while another sends. */
static wchar_t thread_text[64];
static LRESULT thread_length;
static DWORD WINAPI sender(LPVOID unused) {
    (void)unused;
    SendMessageW(window, WM_SETTEXT, 0, (LPARAM)L"from a thread");
    thread_length = SendMessageW(window, WM_GETTEXT, 64, (LPARAM)thread_text);
    SetWindowPos(window, NULL, 5, 6, 120, 80, SWP_NOZORDER | SWP_NOACTIVATE);
    return 0;
}
static void thread_step(void) {
    DWORD id;
    HANDLE thread = CreateThread(NULL, 0, sender, NULL, 0, &id);
    RECT r;
    wchar_t text[64];
    for (;;) {
        MSG m;
        DWORD wait = MsgWaitForMultipleObjects(1, &thread, FALSE, 5000, QS_ALLINPUT);
        while (PeekMessageW(&m, NULL, 0, 0, PM_REMOVE)) DispatchMessageW(&m);
        if (wait == WAIT_OBJECT_0 || wait == WAIT_TIMEOUT) break;
    }
    CloseHandle(thread);
    GetWindowTextW(window, text, 64);
    GetWindowRect(window, &r);
    put("thread text=");
    wput(text);
    field("length", (long)thread_length);
    field("got", wsame(thread_text, L"from a thread"));
    field("posing", posing_seen);
    put(" pos=");
    num(posing_x);
    put(",");
    num(posing_y);
    put(" size=");
    num(posing_cx);
    put("x");
    num(posing_cy);
    put(" rect=");
    num(r.right - r.left);
    put("x");
    num(r.bottom - r.top);
    nl();
}

static void paint_step(void) {
    struct {
        PAINTSTRUCT ps;
        DWORD canary;
    } p;
    HDC dc;
    p.canary = CANARY;
    dc = BeginPaint(window, &p.ps);
    put("paint size=");
    num((long)sizeof p.ps);
    field("hdc", dc != NULL && p.ps.hdc == dc);
    field("canary", p.canary == CANARY);
    EndPaint(window, &p.ps);
    nl();
}

static void gdi_step(void) {
    struct {
        void *bits;
        DWORD canary;
    } b;
    struct {
        BITMAP bm;
        DWORD canary;
    } o;
    struct {
        LOGBRUSH lb;
        DWORD canary;
    } l;
    BITMAPINFO bi;
    HDC dc = CreateCompatibleDC(NULL);
    HBITMAP bitmap, old;
    HBRUSH brush = CreateSolidBrush(RGB(0x12, 0x34, 0x56));
    RECT r = {0, 0, 4, 4};
    int size, brush_size;
    memset(&bi, 0, sizeof bi);
    bi.bmiHeader.biSize = sizeof bi.bmiHeader;
    bi.bmiHeader.biWidth = 4;
    bi.bmiHeader.biHeight = -4;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    b.bits = NULL;
    b.canary = CANARY;
    bitmap = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &b.bits, NULL, 0);
    old = (HBITMAP)SelectObject(dc, bitmap);
    FillRect(dc, &r, brush);
    GdiFlush();
    o.canary = CANARY;
    size = GetObjectW(bitmap, sizeof o.bm, &o.bm);
    l.canary = CANARY;
    brush_size = GetObjectW(brush, sizeof l.lb, &l.lb);
    put("gdi section=");
    num(bitmap != NULL && b.bits != NULL);
    field("canary", b.canary == CANARY);
    put(" pixel=");
    hex(b.bits ? ((DWORD *)b.bits)[5] : 0);
    field("bitmap", size);
    field("bits", o.bm.bmBits == b.bits);
    field("width", o.bm.bmWidth);
    field("bpp", o.bm.bmBitsPixel);
    field("bitmap-canary", o.canary == CANARY);
    field("brush", brush_size);
    put(" color=");
    hex(l.lb.lbColor);
    field("brush-canary", l.canary == CANARY);
    nl();
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteObject(brush);
    DeleteDC(dc);
}

static void control_step(void) {
    HWND button = CreateWindowExW(0, L"BUTTON", L"Check", WS_CHILD | BS_AUTOCHECKBOX, 0, 0, 80, 20, window, (HMENU)11,
                                  instance, NULL);
    HWND edit = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | ES_AUTOHSCROLL, 0, 30, 80, 20, window, (HMENU)12,
                                instance, NULL);
    struct {
        DWORD start, end;
        DWORD canary;
    } s;
    SendMessageW(button, BM_SETCHECK, BST_CHECKED, 0);
    SendMessageW(edit, WM_SETTEXT, 0, (LPARAM)L"abcdefgh");
    SendMessageW(edit, EM_SETSEL, 2, 5);
    s.canary = CANARY;
    SendMessageW(edit, EM_GETSEL, (WPARAM)&s.start, (LPARAM)&s.end);
    put("controls check=");
    num((long)SendMessageW(button, BM_GETCHECK, 0, 0));
    field("id", GetDlgCtrlID(edit));
    field("item", GetDlgItem(window, 12) == edit);
    field("start", (long)s.start);
    field("end", (long)s.end);
    field("canary", s.canary == CANARY);
    field("length", GetWindowTextLengthW(edit));
    nl();
}

static int children;
static BOOL CALLBACK count_child(HWND child, LPARAM lparam) {
    (void)child;
    if (lparam == (LPARAM)&cookie) children++;
    return TRUE;
}
static int timer_hits;
static UINT_PTR timer_id;
static void CALLBACK on_timer(HWND hwnd, UINT message, UINT_PTR id, DWORD time) {
    (void)time;
    if (hwnd == window && message == WM_TIMER) {
        timer_id = id;
        timer_hits++;
    }
}
static void callback_step(void) {
    DWORD start = GetTickCount();
    EnumChildWindows(window, count_child, (LPARAM)&cookie);
    SetTimer(window, 9, 1, on_timer);
    while (!timer_hits && GetTickCount() - start < 5000) {
        MSG m;
        MsgWaitForMultipleObjects(0, NULL, FALSE, 50, QS_ALLINPUT);
        while (PeekMessageW(&m, NULL, 0, 0, PM_REMOVE)) DispatchMessageW(&m);
    }
    KillTimer(window, 9);
    put("callbacks children=");
    num(children);
    field("timer", timer_hits > 0);
    field("id", (long)timer_id);
    nl();
}

static void menu_step(void) {
    HMENU bar = CreateMenu(), popup = CreatePopupMenu();
    wchar_t text[16];
    AppendMenuW(popup, MF_STRING, 101, L"Open");
    AppendMenuW(popup, MF_SEPARATOR, 0, NULL);
    AppendMenuW(popup, MF_STRING, 102, L"Exit");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)popup, L"File");
    put("menu count=");
    num(GetMenuItemCount(popup));
    field("sub", GetSubMenu(bar, 0) == popup);
    field("id", (long)GetMenuItemID(popup, 2));
    GetMenuStringW(popup, 0, text, 16, MF_BYPOSITION);
    put(" text=");
    wput(text);
    field("set", SetMenu(window, bar));
    field("get", GetMenu(window) == bar);
    nl();
}

static LPARAM init_param;
static INT_PTR CALLBACK dialog_proc(HWND dialog, UINT message, WPARAM wparam, LPARAM lparam) {
    (void)wparam;
    if (message == WM_INITDIALOG) {
        init_param = lparam;
        return TRUE;
    }
    if (message == WM_APP + 5) {
        SetWindowLongW(dialog, DWL_MSGRESULT, 77);
        return TRUE;
    }
    return FALSE;
}
static void dialog_step(void) {
    /* DLGTEMPLATE (style, ex style, items, x, y, cx, cy), then empty menu,
     * class and title arrays; DWORD aligned. */
    static DWORD storage[16];
    DLGTEMPLATE *t = (DLGTEMPLATE *)storage;
    HWND dialog;
    t->style = WS_POPUP | WS_CAPTION | DS_MODALFRAME;
    t->dwExtendedStyle = 0;
    t->cdit = 0;
    t->x = 0;
    t->y = 0;
    t->cx = 100;
    t->cy = 50;
    dialog = CreateDialogIndirectParamW(instance, t, window, dialog_proc, 0x1234);
    put("dialog created=");
    num(dialog != NULL);
    field("init", (long)init_param);
    SetWindowLongW(dialog, DWL_USER, 0x5678);
    field("user", GetWindowLongW(dialog, DWL_USER));
    field("dlgproc", GetWindowLongW(dialog, DWL_DLGPROC) == (LONG)dialog_proc);
    field("result", (long)SendMessageW(dialog, WM_APP + 5, 0, 0));
    DestroyWindow(dialog);
    nl();
}

struct step {
    const char *name;
    void (*run)(void);
};
static const struct step steps[] = {
    {"class", class_step},     {"create", create_step}, {"message", message_step}, {"thread", thread_step},
    {"paint", paint_step},     {"gdi", gdi_step},       {"control", control_step}, {"callback", callback_step},
    {"menu", menu_step},       {"dialog", dialog_step},
};
static const char *const expected[] = {
#include "x86gui.expected.h"
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[400];
    int n = line_length(s), k;
    if (n > 399) n = 399;
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
    while (*w && *w == *word) w++, word++;
    return *w == *word;
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    int raw = last_word_is(line, L"raw"), failed = 0, i;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    instance = GetModuleHandleW(NULL);
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
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
            put("X86-B ");
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
    if (window) DestroyWindow(window);
    if (raw) ExitProcess(0);
    put(failed ? "X86-B FAILED\r\n" : "X86-B PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

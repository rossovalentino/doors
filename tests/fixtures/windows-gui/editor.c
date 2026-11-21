/* Doors OS P13 fixture F2: a small Notepad-like Win32 editor.
 *
 * Ordinary Windows SDK C without a CRT, built with the pinned MSVC toolchain in
 * two variants from this one source: wingui-editor.exe builds its menu,
 * accelerators and strings at run time; wingui-editor-rc.exe (EDITOR_RESOURCES,
 * linked with editor.rc) loads the same menu, accelerators, strings, About
 * dialog and version from resources. There are no Door headers, imports or
 * conditional paths: the same executable bytes run on the Windows reference and
 * in Door.
 *
 * Modes (one exact argument):
 *   (none)      interactive editor: menus, accelerators, an edit control,
 *               Open/Save common dialogs, a comctl32 status bar and a
 *               custom-chrome window (View, Ctrl+K; Esc closes it) whose
 *               WM_NCHITTEST provides caption, close box and size grip;
 *               exits 0. Driven by real keyboard and mouse input.
 *   --selftest  non-interactive: types into its own edit control with posted
 *               messages, checks menus/accelerators/strings, reentrant
 *               SendMessage cases, the About dialog and the custom chrome,
 *               saves through GetSaveFileNameW and exits 80
 *   --reopen    relaunch: reopens the saved file through GetOpenFileNameW,
 *               compares the text and exits 81
 * Results are deterministic lines written to standard output (when the process
 * has one) and appended to wingui-editor[-rc].results in the working directory.
 * A failed check writes one EDITOR_FAIL line and exits with its code. Every
 * input is a message to this process's own windows; the self-test never
 * injects system input, changes settings or touches the clipboard, and the
 * dialogs pass OFN_DONTADDTORECENT.
 */
#define UNICODE
#define _UNICODE
#define WINVER 0x0a00
#define _WIN32_WINNT 0x0a00
#include <windows.h>
#include <commdlg.h>
#include <dlgs.h>
#include <commctrl.h>
#include "editor.h"

#ifdef EDITOR_RESOURCES
#define VARIANT "rc"
#define RESULTS_NAME L"wingui-editor-rc.results"
#else
#define VARIANT "code"
#define RESULTS_NAME L"wingui-editor.results"
#endif

#define EXIT_SELFTEST 80
#define EXIT_REOPEN 81
#define TEXT_MAX 32768
#define FILE_MAX 98304

#define WM_OUTER (WM_APP + 1)
#define WM_QUERY (WM_APP + 2)
#define WM_CROSS (WM_APP + 3)
#define WM_SENTINEL (WM_APP + 4)
#define EM_QUERY_PARENT (WM_APP + 5)
#define WM_DESTROY_SELF (WM_APP + 6)

#define CHROME_X 760
#define CHROME_Y 120
#define CHROME_W 320
#define CHROME_H 200
#define CAPTION_H 32
#define CLOSE_W 40
#define GRIP 16
#define CHROME_MIN_W 200
#define CHROME_MIN_H 120

#define SENTINEL 0x0badu

static const WCHAR main_class[] = L"DoorsEditor";
static const WCHAR chrome_class[] = L"DoorsEditorChrome";
static const WCHAR worker_class[] = L"DoorsEditorWorker";
static const WCHAR temp_class[] = L"DoorsEditorTemp";
static const WCHAR doc_name[] = L"doors-editor-\x03bb.txt";
static const WCHAR doc_text[] = L"Doors editor \x03bb \x9580 \x00e9\r\nsecond line";
static const char doc_utf8[] = "Doors editor \xce\xbb \xe9\x96\x80 \xc3\xa9\r\nsecond line";

static HINSTANCE instance;
static HWND main_hwnd, edit_hwnd, status_hwnd, chrome_hwnd, worker_hwnd;
static HACCEL accel;
static WNDPROC edit_original;
static BOOL hooked;
static int chrome_press;
static WCHAR path[MAX_PATH + 64];
static WCHAR dialog_path[MAX_PATH + 64];
static WCHAR directory[MAX_PATH + 64];
static WCHAR hook_path[MAX_PATH + 64];
static WCHAR text_buffer[TEXT_MAX];
static BYTE file_buffer[FILE_MAX];
static volatile LONG sentinel_seen;
static volatile LONG quit_seen;
static volatile LONG en_change;
static volatile LONG about_checks;
static volatile LONG about_attempts;
static int about_button;
static volatile LONG hook_initdone;
static volatile LONG hook_fileok;
static volatile LONG hook_posts;
static volatile LONG parent_destroy_notes;
static volatile LONG cross_insend;
static volatile LONG status_clicks;
static int outer_depth, edit_depth, cross_active;
static DWORD order[16];
static DWORD order_count;

/* ---- no-CRT helpers ---------------------------------------------------- */

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

static void wappend(WCHAR *buffer, DWORD capacity, const WCHAR *text)
{
    DWORD at = wlen(buffer);
    while (*text && at + 1 < capacity)
        buffer[at++] = *text++;
    buffer[at] = 0;
}

static void wcopy(WCHAR *buffer, DWORD capacity, const WCHAR *text)
{
    buffer[0] = 0;
    wappend(buffer, capacity, text);
}

static DWORD crc32(const BYTE *bytes, DWORD count)
{
    DWORD crc = 0xffffffffu, i;
    int bit;
    for (i = 0; i < count; ++i) {
        crc ^= bytes[i];
        for (bit = 0; bit < 8; ++bit)
            crc = (crc >> 1) ^ (0xedb88320u & (0u - (crc & 1u)));
    }
    return ~crc;
}

static const WCHAR *file_part(const WCHAR *full)
{
    const WCHAR *name = full;
    for (; *full; ++full)
        if (*full == L'\\' || *full == L'/')
            name = full + 1;
    return name;
}

/* ---- output -------------------------------------------------------------- */

static HANDLE out_handle;
static HANDLE results_handle;
static char line[1024];
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

static void put_hex(DWORD value)
{
    static const char hex[] = "0123456789abcdef";
    int shift;
    for (shift = 28; shift >= 0; shift -= 4)
        put_char(hex[(value >> shift) & 15]);
}

/* UTF-8 of a short wide string (file names), without a terminator. */
static void put_wide(const WCHAR *text)
{
    char bytes[512];
    int count = WideCharToMultiByte(CP_UTF8, 0, text, (int)wlen(text), bytes, sizeof(bytes), NULL, NULL);
    int i;
    for (i = 0; i < count; ++i)
        put_char(bytes[i]);
}

static void write_all(HANDLE handle, DWORD count)
{
    DWORD written = 0;
    if (!handle || handle == INVALID_HANDLE_VALUE || !count)
        return;
    if (!WriteFile(handle, line, count, &written, NULL) || written != count)
        ExitProcess(90);
}

static void flush_line(void)
{
    DWORD count = line_len;
    line_len = 0;
    write_all(out_handle, count);
    write_all(results_handle, count);
    if (results_handle && results_handle != INVALID_HANDLE_VALUE)
        FlushFileBuffers(results_handle);
}

static __declspec(noreturn) void finish(UINT code)
{
    put("EDITOR_EXIT variant=" VARIANT " code=");
    put_dec(code);
    put("\r\n");
    flush_line();
    ExitProcess(code);
}

static __declspec(noreturn) void fail(UINT code)
{
    DWORD error = GetLastError();
    line_len = 0;
    put("EDITOR_FAIL variant=" VARIANT " code=");
    put_dec(code);
    put(" error=");
    put_dec(error);
    put("\r\n");
    flush_line();
    ExitProcess(code);
}

static void probe(const char *name, DWORD value)
{
    put("EDITOR_PROBE name=");
    put(name);
    put(" value=");
    put_dec(value);
    put("\r\n");
    flush_line();
}

static void order_add(DWORD step)
{
    if (order_count < sizeof(order) / sizeof(order[0]))
        order[order_count++] = step;
}

static void probe_order(const char *name)
{
    DWORD i;
    put("EDITOR_ORDER name=");
    put(name);
    put(" steps=");
    for (i = 0; i < order_count; ++i) {
        if (i)
            put_char(',');
        put_dec(order[i]);
    }
    put("\r\n");
    flush_line();
}

static BOOL order_is(const DWORD *expected, DWORD count)
{
    DWORD i;
    if (order_count != count)
        return FALSE;
    for (i = 0; i < count; ++i)
        if (order[i] != expected[i])
            return FALSE;
    return TRUE;
}

/* ---- strings, menu and accelerators (code and resource variants) --------- */

static void load(UINT id, WCHAR *buffer, int capacity)
{
#ifdef EDITOR_RESOURCES
    if (LoadStringW(instance, id, buffer, capacity) <= 0)
        fail(100);
#else
    const WCHAR *text = id == IDS_APP_TITLE ? L"Doors Editor"
                        : id == IDS_UNTITLED ? L"Untitled"
                        : id == IDS_SAVE_PROMPT ? L"Save changes to "
                                                : L"";
    wcopy(buffer, (DWORD)capacity, text);
#endif
}

#ifndef EDITOR_RESOURCES
static BOOL append_items(HMENU menu, const UINT *ids, const WCHAR *const *labels, int count)
{
    int i;
    for (i = 0; i < count; ++i)
        if (!(ids[i] ? AppendMenuW(menu, MF_STRING, ids[i], labels[i]) : AppendMenuW(menu, MF_SEPARATOR, 0, NULL)))
            return FALSE;
    return TRUE;
}
#endif

static HMENU build_menu(void)
{
#ifdef EDITOR_RESOURCES
    return LoadMenuW(instance, MAKEINTRESOURCEW(IDR_MENU));
#else
    static const UINT file_ids[] = {ID_FILE_NEW, ID_FILE_OPEN, ID_FILE_SAVE, ID_FILE_SAVEAS, 0, ID_FILE_EXIT};
    static const WCHAR *const file_labels[] = {L"&New\tCtrl+N", L"&Open...\tCtrl+O", L"&Save\tCtrl+S",
                                               L"Save &As...\tCtrl+Shift+S", NULL, L"E&xit"};
    static const UINT edit_ids[] = {ID_EDIT_UNDO,   0, ID_EDIT_CUT, ID_EDIT_COPY, ID_EDIT_PASTE, ID_EDIT_DELETE, 0,
                                    ID_EDIT_SELECTALL};
    static const WCHAR *const edit_labels[] = {L"&Undo\tCtrl+Z", NULL,           L"Cu&t\tCtrl+X", L"&Copy\tCtrl+C",
                                               L"&Paste\tCtrl+V", L"De&lete\tDel", NULL, L"Select &All\tCtrl+A"};
    static const UINT view_ids[] = {ID_VIEW_CHROME};
    static const WCHAR *const view_labels[] = {L"Custom &chrome window\tCtrl+K"};
    static const UINT help_ids[] = {ID_HELP_ABOUT};
    static const WCHAR *const help_labels[] = {L"&About Doors Editor"};
    HMENU bar = CreateMenu(), file = CreatePopupMenu(), edit = CreatePopupMenu(), view = CreatePopupMenu(),
          help = CreatePopupMenu();
    if (!bar || !file || !edit || !view || !help || !append_items(file, file_ids, file_labels, 6)
        || !append_items(edit, edit_ids, edit_labels, 8) || !append_items(view, view_ids, view_labels, 1)
        || !append_items(help, help_ids, help_labels, 1) || !AppendMenuW(bar, MF_POPUP, (UINT_PTR)file, L"&File")
        || !AppendMenuW(bar, MF_POPUP, (UINT_PTR)edit, L"&Edit") || !AppendMenuW(bar, MF_POPUP, (UINT_PTR)view, L"&View")
        || !AppendMenuW(bar, MF_POPUP, (UINT_PTR)help, L"&Help"))
        return NULL;
    return bar;
#endif
}

static const ACCEL accel_table[] = {
    {FVIRTKEY | FCONTROL, 'N', ID_FILE_NEW},          {FVIRTKEY | FCONTROL, 'O', ID_FILE_OPEN},
    {FVIRTKEY | FCONTROL, 'S', ID_FILE_SAVE},         {FVIRTKEY | FCONTROL | FSHIFT, 'S', ID_FILE_SAVEAS},
    {FVIRTKEY | FCONTROL, 'A', ID_EDIT_SELECTALL},    {FVIRTKEY | FCONTROL, 'K', ID_VIEW_CHROME},
};

static HACCEL build_accel(void)
{
#ifdef EDITOR_RESOURCES
    return LoadAcceleratorsW(instance, MAKEINTRESOURCEW(IDR_ACCEL));
#else
    return CreateAcceleratorTableW((ACCEL *)accel_table, sizeof(accel_table) / sizeof(accel_table[0]));
#endif
}

/* ---- document --------------------------------------------------------------- */

static void update_title(void)
{
    WCHAR title[MAX_PATH + 96], app[64], untitled[64];
    load(IDS_APP_TITLE, app, 64);
    load(IDS_UNTITLED, untitled, 64);
    wcopy(title, MAX_PATH + 96, path[0] ? file_part(path) : untitled);
    wappend(title, MAX_PATH + 96, L" - ");
    wappend(title, MAX_PATH + 96, app);
    SetWindowTextW(main_hwnd, title);
}

static void set_status(const WCHAR *text)
{
    if (status_hwnd)
        SendMessageW(status_hwnd, SB_SETTEXTW, 0, (LPARAM)text);
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
            /* The folder view is ready: name the document, then press OK. A press
               consumed by a pending navigation is repeated, bounded. */
            ++hook_posts;
            SendMessageW(dialog, CDM_SETCONTROLTEXT, GetDlgItem(dialog, cmb13) ? cmb13 : edt1, (LPARAM)hook_path);
            PostMessageW(dialog, WM_COMMAND, MAKEWPARAM(IDOK, BN_CLICKED), (LPARAM)GetDlgItem(dialog, IDOK));
        } else if (note->hdr.code == CDN_FILEOK) {
            InterlockedIncrement(&hook_fileok);
        }
    }
    return 0;
}

/* Open or Save As through the common dialog; on success `path` names the file. */
static BOOL file_dialog(BOOL save)
{
    OPENFILENAMEW dialog;
    BOOL ok;
    zero(&dialog, sizeof(dialog));
    if (!GetCurrentDirectoryW(MAX_PATH + 64, directory))
        fail(110);
    wcopy(dialog_path, MAX_PATH + 64, path[0] ? file_part(path) : L"");
    dialog.lStructSize = sizeof(dialog);
    dialog.hwndOwner = main_hwnd;
    dialog.hInstance = instance;
    dialog.lpstrFilter = L"Text documents (*.txt)\0*.txt\0All files (*.*)\0*.*\0";
    dialog.nFilterIndex = 1;
    dialog.lpstrFile = dialog_path;
    dialog.nMaxFile = MAX_PATH + 64;
    dialog.lpstrInitialDir = directory;
    dialog.lpstrDefExt = L"txt";
    dialog.Flags = OFN_EXPLORER | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT | OFN_PATHMUSTEXIST
                   | (save ? (hooked ? 0 : OFN_OVERWRITEPROMPT) : OFN_FILEMUSTEXIST);
    /* The hook types the full target path, so the result does not depend on
       the initial folder the system chooses. */
    if (hooked) {
        DWORD length;
        wcopy(hook_path, MAX_PATH + 64, directory);
        length = wlen(hook_path);
        if (!length || hook_path[length - 1] != L'\\')
            wappend(hook_path, MAX_PATH + 64, L"\\");
        wappend(hook_path, MAX_PATH + 64, doc_name);
        dialog.Flags |= OFN_ENABLEHOOK;
        dialog.lpfnHook = file_hook;
        hook_initdone = 0;
        hook_fileok = 0;
        hook_posts = 0;
    }
    ok = save ? GetSaveFileNameW(&dialog) : GetOpenFileNameW(&dialog);
    put("EDITOR_DIALOG kind=");
    put(save ? "save" : "open");
    put(" result=");
    put_dec(ok ? 1 : 0);
    if (ok) {
        put(" name=");
        put_wide(file_part(dialog_path));
        put(" offset_ok=");
        put_dec(dialog.nFileOffset && weq(dialog_path + dialog.nFileOffset, file_part(dialog_path)) ? 1 : 0);
    } else {
        put(" error=");
        put_dec(CommDlgExtendedError());
    }
    put("\r\n");
    flush_line();
    if (!ok)
        return FALSE;
    if (hooked && (hook_initdone != 1 || hook_fileok != 1))
        fail(111);
    wcopy(path, MAX_PATH + 64, dialog_path);
    return TRUE;
}

static BOOL save_file(void)
{
    HANDLE file;
    DWORD chars, bytes, written = 0;
    chars = (DWORD)SendMessageW(edit_hwnd, WM_GETTEXT, TEXT_MAX, (LPARAM)text_buffer);
    bytes = chars ? (DWORD)WideCharToMultiByte(CP_UTF8, 0, text_buffer, (int)chars, (char *)file_buffer, FILE_MAX,
                                               NULL, NULL)
                  : 0;
    if (chars && !bytes)
        fail(120);
    file = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) {
        put("EDITOR_SAVE result=0 error=");
        put_dec(GetLastError());
        put("\r\n");
        flush_line();
        return FALSE;
    }
    if ((bytes && (!WriteFile(file, file_buffer, bytes, &written, NULL) || written != bytes))
        || !FlushFileBuffers(file) || !CloseHandle(file))
        fail(121);
    SendMessageW(edit_hwnd, EM_SETMODIFY, FALSE, 0);
    update_title();
    set_status(L"Saved");
    put("EDITOR_SAVE result=1 name=");
    put_wide(file_part(path));
    put(" chars=");
    put_dec(chars);
    put(" bytes=");
    put_dec(bytes);
    put(" crc=");
    put_hex(crc32(file_buffer, bytes));
    put("\r\n");
    flush_line();
    return TRUE;
}

static BOOL load_file(void)
{
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    DWORD size, actual = 0, skip = 0;
    int chars;
    if (file == INVALID_HANDLE_VALUE) {
        put("EDITOR_OPEN result=0 error=");
        put_dec(GetLastError());
        put("\r\n");
        flush_line();
        return FALSE;
    }
    size = GetFileSize(file, NULL);
    if (size == INVALID_FILE_SIZE || size >= FILE_MAX || (size && !ReadFile(file, file_buffer, size, &actual, NULL))
        || actual != size || !CloseHandle(file))
        fail(130);
    if (size >= 3 && file_buffer[0] == 0xef && file_buffer[1] == 0xbb && file_buffer[2] == 0xbf)
        skip = 3;
    chars = size - skip ? MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, (const char *)file_buffer + skip,
                                              (int)(size - skip), text_buffer, TEXT_MAX - 1)
                        : 0;
    if (size - skip && chars <= 0)
        fail(131);
    text_buffer[chars] = 0;
    if (!SetWindowTextW(edit_hwnd, text_buffer))
        fail(132);
    SendMessageW(edit_hwnd, EM_SETMODIFY, FALSE, 0);
    update_title();
    set_status(L"Opened");
    put("EDITOR_OPEN result=1 name=");
    put_wide(file_part(path));
    put(" bytes=");
    put_dec(size);
    put(" chars=");
    put_dec((DWORD)chars);
    put(" crc=");
    put_hex(crc32(file_buffer, size));
    put("\r\n");
    flush_line();
    return TRUE;
}

static BOOL save(BOOL as)
{
    if ((as || !path[0]) && !file_dialog(TRUE))
        return FALSE;
    return save_file();
}

/* Yes saves, No discards, Cancel keeps the document open. */
static BOOL confirm_discard(void)
{
    WCHAR prompt[MAX_PATH + 96], app[64], untitled[64];
    int answer;
    if (!SendMessageW(edit_hwnd, EM_GETMODIFY, 0, 0))
        return TRUE;
    load(IDS_SAVE_PROMPT, prompt, MAX_PATH + 96);
    load(IDS_UNTITLED, untitled, 64);
    load(IDS_APP_TITLE, app, 64);
    wappend(prompt, MAX_PATH + 96, path[0] ? file_part(path) : untitled);
    wappend(prompt, MAX_PATH + 96, L"?");
    answer = MessageBoxW(main_hwnd, prompt, app, MB_YESNOCANCEL | MB_ICONWARNING);
    if (answer == IDYES)
        return save(FALSE);
    return answer == IDNO;
}

/* ---- About -------------------------------------------------------------------- */

#ifdef EDITOR_RESOURCES
static INT_PTR CALLBACK about_proc(HWND dialog, UINT message, WPARAM wparam, LPARAM lparam)
{
    (void)lparam;
    if (message == WM_INITDIALOG)
        return TRUE;
    if (message == WM_COMMAND && (LOWORD(wparam) == IDOK || LOWORD(wparam) == IDCANCEL)) {
        EndDialog(dialog, LOWORD(wparam));
        return TRUE;
    }
    return FALSE;
}
#endif

static INT_PTR about(void)
{
#ifdef EDITOR_RESOURCES
    return DialogBoxParamW(instance, MAKEINTRESOURCEW(IDD_ABOUT), main_hwnd, about_proc, 0);
#else
    return MessageBoxW(main_hwnd, L"Doors Editor (code variant)", L"About Doors Editor", MB_OK | MB_ICONINFORMATION);
#endif
}

/* ---- custom chrome --------------------------------------------------------------- */

static void report_chrome(HWND hwnd, const char *event)
{
    RECT rect;
    GetWindowRect(hwnd, &rect);
    put("EDITOR_CHROME event=");
    put(event);
    put(" x=");
    put_dec((DWORD)rect.left);
    put(" y=");
    put_dec((DWORD)rect.top);
    put(" w=");
    put_dec((DWORD)(rect.right - rect.left));
    put(" h=");
    put_dec((DWORD)(rect.bottom - rect.top));
    put("\r\n");
    flush_line();
}

static BOOL in_close_box(HWND hwnd, int x, int y)
{
    RECT client;
    GetClientRect(hwnd, &client);
    return y >= 0 && y < CAPTION_H && x >= client.right - CLOSE_W && x < client.right;
}

static void paint_chrome(HWND hwnd)
{
    PAINTSTRUCT paint;
    RECT client, part;
    HBRUSH body, caption, close;
    HPEN white, old_pen;
    HDC dc = BeginPaint(hwnd, &paint);
    if (!dc)
        return;
    GetClientRect(hwnd, &client);
    body = CreateSolidBrush(RGB(0xf3, 0xf3, 0xf3));
    caption = CreateSolidBrush(RGB(0x20, 0x50, 0x90));
    close = CreateSolidBrush(RGB(0xc4, 0x2b, 0x1c));
    white = CreatePen(PS_SOLID, 2, RGB(0xff, 0xff, 0xff));
    FillRect(dc, &client, body);
    SetRect(&part, 0, 0, client.right, CAPTION_H);
    FillRect(dc, &part, caption);
    SetRect(&part, client.right - CLOSE_W, 0, client.right, CAPTION_H);
    FillRect(dc, &part, close);
    old_pen = SelectObject(dc, white);
    MoveToEx(dc, client.right - CLOSE_W + 14, 10, NULL);
    LineTo(dc, client.right - 14, CAPTION_H - 10);
    MoveToEx(dc, client.right - 14, 10, NULL);
    LineTo(dc, client.right - CLOSE_W + 14, CAPTION_H - 10);
    SelectObject(dc, GetStockObject(BLACK_PEN));
    MoveToEx(dc, client.right - 4, client.bottom - GRIP, NULL);
    LineTo(dc, client.right - GRIP, client.bottom - 4);
    MoveToEx(dc, client.right - 4, client.bottom - 9, NULL);
    LineTo(dc, client.right - 9, client.bottom - 4);
    SelectObject(dc, old_pen);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, RGB(0xff, 0xff, 0xff));
    TextOutW(dc, 10, 8, L"Doors custom chrome", 19);
    SetTextColor(dc, RGB(0x20, 0x20, 0x20));
    TextOutW(dc, 10, CAPTION_H + 12, L"Drag the caption, size from the corner.", 39);
    DeleteObject(body);
    DeleteObject(caption);
    DeleteObject(close);
    DeleteObject(white);
    EndPaint(hwnd, &paint);
}

static LRESULT CALLBACK chrome_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    switch (message) {
    case WM_NCCALCSIZE:
        /* No standard frame: the client area is the whole window (wParam FALSE
           at creation, TRUE afterwards). */
        return 0;
    case WM_NCHITTEST: {
        RECT rect;
        int x = (int)(short)LOWORD(lparam), y = (int)(short)HIWORD(lparam), w, h;
        GetWindowRect(hwnd, &rect);
        x -= rect.left;
        y -= rect.top;
        w = rect.right - rect.left;
        h = rect.bottom - rect.top;
        if (x < 0 || y < 0 || x >= w || y >= h)
            return HTNOWHERE;
        if (x >= w - GRIP && y >= h - GRIP)
            return HTBOTTOMRIGHT;
        if (y < CAPTION_H)
            return x >= w - CLOSE_W ? HTCLIENT : HTCAPTION;
        return HTCLIENT;
    }
    case WM_GETMINMAXINFO:
        ((MINMAXINFO *)lparam)->ptMinTrackSize.x = CHROME_MIN_W;
        ((MINMAXINFO *)lparam)->ptMinTrackSize.y = CHROME_MIN_H;
        return 0;
    case WM_PAINT:
        paint_chrome(hwnd);
        return 0;
    case WM_LBUTTONDOWN:
        chrome_press = in_close_box(hwnd, (int)(short)LOWORD(lparam), (int)(short)HIWORD(lparam));
        return 0;
    case WM_LBUTTONUP:
        if (chrome_press && in_close_box(hwnd, (int)(short)LOWORD(lparam), (int)(short)HIWORD(lparam))) {
            chrome_press = 0;
            report_chrome(hwnd, "close");
            DestroyWindow(hwnd);
        }
        chrome_press = 0;
        return 0;
    case WM_EXITSIZEMOVE:
        report_chrome(hwnd, "moved");
        return 0;
    case WM_KEYDOWN:
        if (wparam == VK_ESCAPE) {
            report_chrome(hwnd, "escape");
            DestroyWindow(hwnd);
            return 0;
        }
        break;
    case WM_DESTROY:
        chrome_hwnd = NULL;
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static void open_chrome(int show)
{
    if (chrome_hwnd) {
        SetForegroundWindow(chrome_hwnd);
        return;
    }
    chrome_hwnd = CreateWindowExW(0, chrome_class, L"Doors custom chrome", WS_POPUP | WS_THICKFRAME, CHROME_X, CHROME_Y,
                                  CHROME_W, CHROME_H, main_hwnd, NULL, instance, NULL);
    if (!chrome_hwnd)
        fail(140);
    ShowWindow(chrome_hwnd, show);
    UpdateWindow(chrome_hwnd);
    report_chrome(chrome_hwnd, "open");
}

/* ---- main window ------------------------------------------------------------------ */

static LRESULT CALLBACK edit_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == EM_QUERY_PARENT) {
        LRESULT reply;
        ++edit_depth;
        order_add(10 + (DWORD)edit_depth);
        reply = SendMessageW(GetParent(hwnd), WM_QUERY, wparam, 0);
        order_add(20 + (DWORD)edit_depth);
        --edit_depth;
        return reply + 1;
    }
    if (message == EM_GETLINECOUNT && edit_depth)
        order_add(30 + (DWORD)edit_depth);
    if (message == WM_GETTEXTLENGTH && cross_active)
        order_add(65);
    return CallWindowProcW(edit_original, hwnd, message, wparam, lparam);
}

/* The status bar sizes itself on WM_SIZE; the edit control fills the rest. */
static void layout(int width, int height)
{
    RECT status;
    int bar = 0;
    if (status_hwnd) {
        SendMessageW(status_hwnd, WM_SIZE, 0, 0);
        if (GetWindowRect(status_hwnd, &status))
            bar = status.bottom - status.top;
    }
    if (edit_hwnd)
        MoveWindow(edit_hwnd, 0, 0, width, height > bar ? height - bar : 0, TRUE);
}

static LRESULT command(HWND hwnd, WPARAM wparam, LPARAM lparam)
{
    switch (LOWORD(wparam)) {
    case ID_EDITOR_EDIT:
        if (HIWORD(wparam) == EN_CHANGE && (HWND)lparam == edit_hwnd) {
            InterlockedIncrement(&en_change);
            if (outer_depth)
                order_add(2);
        }
        return 0;
    case ID_FILE_NEW:
        if (confirm_discard()) {
            SetWindowTextW(edit_hwnd, L"");
            SendMessageW(edit_hwnd, EM_SETMODIFY, FALSE, 0);
            path[0] = 0;
            update_title();
            put("EDITOR_NEW\r\n");
            flush_line();
        }
        return 0;
    case ID_FILE_OPEN:
        if (confirm_discard() && file_dialog(FALSE))
            load_file();
        return 0;
    case ID_FILE_SAVE:
        save(FALSE);
        return 0;
    case ID_FILE_SAVEAS:
        save(TRUE);
        return 0;
    case ID_FILE_EXIT:
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        return 0;
    case ID_EDIT_UNDO:
        SendMessageW(edit_hwnd, EM_UNDO, 0, 0);
        return 0;
    case ID_EDIT_CUT:
        SendMessageW(edit_hwnd, WM_CUT, 0, 0);
        return 0;
    case ID_EDIT_COPY:
        SendMessageW(edit_hwnd, WM_COPY, 0, 0);
        return 0;
    case ID_EDIT_PASTE:
        SendMessageW(edit_hwnd, WM_PASTE, 0, 0);
        return 0;
    case ID_EDIT_DELETE:
        SendMessageW(edit_hwnd, WM_CLEAR, 0, 0);
        return 0;
    case ID_EDIT_SELECTALL:
        SendMessageW(edit_hwnd, EM_SETSEL, 0, -1);
        return 0;
    case ID_VIEW_CHROME:
        open_chrome(hooked ? SW_SHOWNOACTIVATE : SW_SHOW);
        return 0;
    case ID_HELP_ABOUT: {
        INT_PTR result = about();
        put("EDITOR_ABOUT result=");
        put_dec((DWORD)result);
        put("\r\n");
        flush_line();
        return 0;
    }
    }
    return DefWindowProcW(hwnd, WM_COMMAND, wparam, lparam);
}

static LRESULT CALLBACK main_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    switch (message) {
    case WM_CREATE:
        edit_hwnd = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", L"",
                                    WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN
                                        | ES_NOHIDESEL,
                                    0, 0, 0, 0, hwnd, (HMENU)(UINT_PTR)ID_EDITOR_EDIT, instance, NULL);
        status_hwnd = CreateWindowExW(0, STATUSCLASSNAMEW, L"", WS_CHILD | WS_VISIBLE, 0, 0, 0, 0, hwnd,
                                      (HMENU)(UINT_PTR)ID_EDITOR_STATUS, instance, NULL);
        if (!edit_hwnd || !status_hwnd)
            return -1;
        {
            static const int parts[2] = {240, -1};
            if (!SendMessageW(status_hwnd, SB_SETPARTS, 2, (LPARAM)parts)
                || !SendMessageW(status_hwnd, SB_SETTEXTW, 0, (LPARAM)L"Ready")
                || !SendMessageW(status_hwnd, SB_SETTEXTW, 1, (LPARAM)L"UTF-8"))
                return -1;
        }
        edit_original = (WNDPROC)SetWindowLongPtrW(edit_hwnd, GWLP_WNDPROC, (LONG_PTR)edit_proc);
        SendMessageW(edit_hwnd, EM_LIMITTEXT, TEXT_MAX - 1, 0);
        return edit_original ? 0 : -1;
    case WM_SIZE:
        layout(LOWORD(lparam), HIWORD(lparam));
        return 0;
    case WM_SETFOCUS:
        SetFocus(edit_hwnd);
        return 0;
    case WM_INITMENUPOPUP: {
        DWORD start = 0, end = 0;
        HMENU popup = (HMENU)wparam;
        SendMessageW(edit_hwnd, EM_GETSEL, (WPARAM)&start, (LPARAM)&end);
        EnableMenuItem(popup, ID_EDIT_UNDO,
                       MF_BYCOMMAND | (SendMessageW(edit_hwnd, EM_CANUNDO, 0, 0) ? MF_ENABLED : MF_GRAYED));
        EnableMenuItem(popup, ID_EDIT_CUT, MF_BYCOMMAND | (start != end ? MF_ENABLED : MF_GRAYED));
        EnableMenuItem(popup, ID_EDIT_COPY, MF_BYCOMMAND | (start != end ? MF_ENABLED : MF_GRAYED));
        EnableMenuItem(popup, ID_EDIT_DELETE, MF_BYCOMMAND | (start != end ? MF_ENABLED : MF_GRAYED));
        return 0;
    }
    case WM_COMMAND:
        return command(hwnd, wparam, lparam);
    case WM_NOTIFY: {
        const NMHDR *note = (const NMHDR *)lparam;
        if (note->idFrom == ID_EDITOR_STATUS && note->hwndFrom == status_hwnd && note->code == NM_CLICK)
            InterlockedIncrement(&status_clicks);
        return 0;
    }
    case WM_PARENTNOTIFY:
        if (LOWORD(wparam) == WM_DESTROY)
            InterlockedIncrement(&parent_destroy_notes);
        return 0;
    case WM_CLOSE:
        if (confirm_discard())
            DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        if (chrome_hwnd)
            DestroyWindow(chrome_hwnd);
        PostQuitMessage(0);
        return 0;
    case WM_SENTINEL:
        InterlockedIncrement(&sentinel_seen);
        return 0;
    case WM_OUTER: {
        LONG before = en_change;
        DWORD end = (DWORD)SendMessageW(edit_hwnd, WM_GETTEXTLENGTH, 0, 0);
        ++outer_depth;
        order_add(1);
        SendMessageW(edit_hwnd, EM_SETSEL, end, end);
        SendMessageW(edit_hwnd, EM_REPLACESEL, TRUE, (LPARAM)L"!");
        order_add(3);
        --outer_depth;
        return en_change - before;
    }
    case WM_QUERY: {
        LRESULT lines;
        order_add(40);
        lines = SendMessageW(edit_hwnd, EM_GETLINECOUNT, 0, 0);
        order_add(50);
        return lines * 100 + (LRESULT)wparam;
    }
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static LRESULT CALLBACK worker_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == WM_CROSS) {
        LRESULT length;
        order_add(60);
        cross_insend = (LONG)InSendMessageEx(NULL);
        length = SendMessageW(edit_hwnd, WM_GETTEXTLENGTH, 0, 0);
        order_add(70);
        return length + 1000;
    }
    if (message == WM_DESTROY) {
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static DWORD WINAPI worker_main(LPVOID ready)
{
    MSG message;
    worker_hwnd = CreateWindowExW(0, worker_class, L"worker", WS_POPUP, 0, 0, 16, 16, HWND_MESSAGE, NULL, instance,
                                  NULL);
    SetEvent((HANDLE)ready);
    if (!worker_hwnd)
        return 1;
    while (GetMessageW(&message, NULL, 0, 0) > 0)
        DispatchMessageW(&message);
    return 3;
}

static LRESULT CALLBACK temp_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam)
{
    if (message == WM_DESTROY_SELF) {
        if (!DestroyWindow(hwnd))
            return 0;
        return 0x55 + (IsWindow(hwnd) ? 1 : 0);
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

static void register_classes(void)
{
    WNDCLASSEXW window;
    zero(&window, sizeof(window));
    window.cbSize = sizeof(window);
    window.style = CS_HREDRAW | CS_VREDRAW;
    window.lpfnWndProc = main_proc;
    window.hInstance = instance;
    window.hCursor = LoadCursorW(NULL, IDC_ARROW);
    window.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    window.lpszClassName = main_class;
    if (!RegisterClassExW(&window))
        fail(101);
    window.lpfnWndProc = chrome_proc;
    window.hbrBackground = NULL;
    window.lpszClassName = chrome_class;
    if (!RegisterClassExW(&window))
        fail(102);
    window.style = 0;
    window.lpfnWndProc = worker_proc;
    window.hCursor = NULL;
    window.lpszClassName = worker_class;
    if (!RegisterClassExW(&window))
        fail(103);
    window.lpfnWndProc = temp_proc;
    window.lpszClassName = temp_class;
    if (!RegisterClassExW(&window))
        fail(104);
}

static void create_main(int show)
{
    HMENU menu = build_menu();
    accel = build_accel();
    if (!menu || !accel)
        fail(105);
    main_hwnd = CreateWindowExW(0, main_class, L"", WS_OVERLAPPEDWINDOW, 64, 64, 640, 440, NULL, menu, instance, NULL);
    if (!main_hwnd)
        fail(106);
    update_title();
    ShowWindow(main_hwnd, show);
    UpdateWindow(main_hwnd);
}

/* ---- self-test ---------------------------------------------------------------------- */

static BOOL pump_once(DWORD wait)
{
    MSG message;
    if (PeekMessageW(&message, NULL, 0, 0, PM_REMOVE)) {
        if (message.message == WM_QUIT) {
            quit_seen = 1;
            return TRUE;
        }
        if (!TranslateAcceleratorW(main_hwnd, accel, &message)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
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

static void drain(UINT code)
{
    sentinel_seen = 0;
    if (!PostMessageW(main_hwnd, WM_SENTINEL, 0, 0))
        fail(code);
    pump_until(&sentinel_seen, 1, 3000, code);
}

static void post_chars(const WCHAR *text)
{
    while (*text)
        if (!PostMessageW(edit_hwnd, WM_CHAR, *text++, 1))
            fail(150);
}

static BOOL edit_is(const WCHAR *expected)
{
    DWORD count = (DWORD)SendMessageW(edit_hwnd, WM_GETTEXT, TEXT_MAX, (LPARAM)text_buffer);
    return count == wlen(expected) && weq(text_buffer, expected);
}

static BOOL menu_item(HMENU menu, int position, UINT id, const WCHAR *label)
{
    WCHAR text[64];
    if (!id)
        return GetMenuItemID(menu, position) == 0 && (GetMenuState(menu, (UINT)position, MF_BYPOSITION) & MF_SEPARATOR);
    return GetMenuItemID(menu, position) == id
           && GetMenuStringW(menu, (UINT)position, text, 64, MF_BYPOSITION) == (int)wlen(label) && weq(text, label);
}

static void check_menu(void)
{
    HMENU bar = GetMenu(main_hwnd), file, edit, view, help;
    WCHAR text[64];
    if (!bar || GetMenuItemCount(bar) != 4 || GetMenuStringW(bar, 0, text, 64, MF_BYPOSITION) != 5
        || !weq(text, L"&File") || GetMenuStringW(bar, 2, text, 64, MF_BYPOSITION) != 5 || !weq(text, L"&View"))
        fail(160);
    file = GetSubMenu(bar, 0);
    edit = GetSubMenu(bar, 1);
    view = GetSubMenu(bar, 2);
    help = GetSubMenu(bar, 3);
    if (GetMenuItemCount(file) != 6 || GetMenuItemCount(edit) != 8 || GetMenuItemCount(view) != 1
        || GetMenuItemCount(help) != 1)
        fail(161);
    if (!menu_item(file, 0, ID_FILE_NEW, L"&New\tCtrl+N") || !menu_item(file, 1, ID_FILE_OPEN, L"&Open...\tCtrl+O")
        || !menu_item(file, 2, ID_FILE_SAVE, L"&Save\tCtrl+S")
        || !menu_item(file, 3, ID_FILE_SAVEAS, L"Save &As...\tCtrl+Shift+S") || !menu_item(file, 4, 0, NULL)
        || !menu_item(file, 5, ID_FILE_EXIT, L"E&xit"))
        fail(162);
    if (!menu_item(edit, 0, ID_EDIT_UNDO, L"&Undo\tCtrl+Z") || !menu_item(edit, 1, 0, NULL)
        || !menu_item(edit, 2, ID_EDIT_CUT, L"Cu&t\tCtrl+X") || !menu_item(edit, 3, ID_EDIT_COPY, L"&Copy\tCtrl+C")
        || !menu_item(edit, 4, ID_EDIT_PASTE, L"&Paste\tCtrl+V") || !menu_item(edit, 5, ID_EDIT_DELETE, L"De&lete\tDel")
        || !menu_item(edit, 6, 0, NULL) || !menu_item(edit, 7, ID_EDIT_SELECTALL, L"Select &All\tCtrl+A"))
        fail(163);
    if (!menu_item(view, 0, ID_VIEW_CHROME, L"Custom &chrome window\tCtrl+K")
        || !menu_item(help, 0, ID_HELP_ABOUT, L"&About Doors Editor"))
        fail(164);
}

static void check_accelerators(void)
{
    ACCEL entries[8];
    int i, count = CopyAcceleratorTableW(accel, NULL, 0);
    if (count != 6 || CopyAcceleratorTableW(accel, entries, 8) != 6)
        fail(170);
    for (i = 0; i < 6; ++i)
        if ((entries[i].fVirt & 0x1f) != accel_table[i].fVirt || entries[i].key != accel_table[i].key
            || entries[i].cmd != accel_table[i].cmd)
            fail(171);
    probe("accelerators", (DWORD)count);
}

static void check_strings(void)
{
    WCHAR text[96];
    load(IDS_APP_TITLE, text, 96);
    if (!weq(text, L"Doors Editor"))
        fail(172);
    load(IDS_SAVE_PROMPT, text, 96);
    if (!weq(text, L"Save changes to "))
        fail(173);
    if (GetWindowTextW(main_hwnd, text, 96) != 23 || !weq(text, L"Untitled - Doors Editor"))
        fail(174);
}

/* The comctl32 status bar: parts, text and an NM_CLICK notification. */
static void check_status(void)
{
    WCHAR text[32];
    int parts[4];
    LRESULT length;
    WCHAR klass[32];
    if (GetClassNameW(status_hwnd, klass, 32) != 18 || !weq(klass, STATUSCLASSNAMEW))
        fail(175);
    if (SendMessageW(status_hwnd, SB_GETPARTS, 4, (LPARAM)parts) != 2 || parts[0] != 240)
        fail(176);
    probe("status_last_part", (DWORD)parts[1]);
    length = SendMessageW(status_hwnd, SB_GETTEXTW, 1, (LPARAM)text);
    if (LOWORD(length) != 5 || !weq(text, L"UTF-8"))
        fail(177);
    status_clicks = 0;
    if (!PostMessageW(status_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(10, 5))
        || !PostMessageW(status_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(10, 5)))
        fail(178);
    drain(179);
    probe("status_clicks", (DWORD)status_clicks);
    if (status_clicks != 1)
        fail(188);
}

static void typing(void)
{
    DWORD start = 0, end = 0, length = wlen(doc_text);
    post_chars(L"Doors editor \x03bb \x9580 \x00e9\rsecond line");
    drain(180);
    if (!edit_is(doc_text) || !SendMessageW(edit_hwnd, EM_GETMODIFY, 0, 0)
        || SendMessageW(edit_hwnd, EM_GETLINECOUNT, 0, 0) != 2)
        fail(181);
    probe("en_change_typed", (DWORD)en_change);
    /* An accelerator-originated command (HIWORD 1) selects all. */
    if (!PostMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_EDIT_SELECTALL, 1), 0))
        fail(182);
    drain(183);
    SendMessageW(edit_hwnd, EM_GETSEL, (WPARAM)&start, (LPARAM)&end);
    if (start != 0 || end != length)
        fail(184);
    /* Delete the last character through the menu command, then retype it. */
    SendMessageW(edit_hwnd, EM_SETSEL, length - 1, length);
    SendMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_EDIT_DELETE, 0), 0);
    if (SendMessageW(edit_hwnd, WM_GETTEXTLENGTH, 0, 0) != (LRESULT)(length - 1))
        fail(185);
    post_chars(L"e");
    drain(186);
    if (!edit_is(doc_text))
        fail(187);
}

static void reentrancy(void)
{
    static const DWORD outer_expected[] = {1, 2, 3};
    static const DWORD query_expected[] = {11, 40, 31, 50, 21};
    static const DWORD cross_expected[] = {60, 65, 70};
    HANDLE ready, thread;
    DWORD id, code;
    LRESULT result;
    HWND temp;

    /* A sent message whose handler sends to a child, which notifies the parent
       synchronously inside the outer call. */
    order_count = 0;
    result = SendMessageW(main_hwnd, WM_OUTER, 0, 0);
    probe_order("outer");
    if (result != 1 || !order_is(outer_expected, 3))
        fail(190);
    SendMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_EDIT_UNDO, 0), 0);
    if (!edit_is(doc_text))
        fail(191);

    /* The subclassed edit control sends to its parent, which sends back to the
       edit control while that control is still inside its own message. */
    order_count = 0;
    result = SendMessageW(edit_hwnd, EM_QUERY_PARENT, 7, 0);
    probe_order("query");
    if (result != 2 * 100 + 7 + 1 || !order_is(query_expected, 5))
        fail(192);

    /* Cross-thread: the worker's handler sends back to this thread's edit
       control while this thread waits in SendMessageW. */
    ready = CreateEventW(NULL, TRUE, FALSE, NULL);
    thread = ready ? CreateThread(NULL, 0, worker_main, ready, 0, &id) : NULL;
    if (!thread || WaitForSingleObject(ready, 5000) != WAIT_OBJECT_0 || !worker_hwnd)
        fail(193);
    order_count = 0;
    cross_active = 1;
    result = SendMessageW(worker_hwnd, WM_CROSS, 0, 0);
    cross_active = 0;
    probe_order("cross");
    probe("cross_insend", (DWORD)cross_insend);
    if (result != (LRESULT)wlen(doc_text) + 1000 || !order_is(cross_expected, 3) || !(cross_insend & ISMEX_SEND))
        fail(194);
    if (!PostMessageW(worker_hwnd, WM_CLOSE, 0, 0) || WaitForSingleObject(thread, 5000) != WAIT_OBJECT_0
        || !GetExitCodeThread(thread, &code) || code != 3 || IsWindow(worker_hwnd))
        fail(195);
    CloseHandle(thread);
    CloseHandle(ready);

    /* A destination that destroys itself inside the sent message still returns
       its result; later sends to it fail with ERROR_INVALID_WINDOW_HANDLE. */
    parent_destroy_notes = 0;
    temp = CreateWindowExW(0, temp_class, L"temp", WS_CHILD, 0, 0, 10, 10, main_hwnd, (HMENU)(UINT_PTR)77, instance,
                           NULL);
    if (!temp)
        fail(196);
    result = SendMessageW(temp, WM_DESTROY_SELF, 0, 0);
    probe("destroy_self_result", (DWORD)result);
    probe("destroy_self_parentnotify", (DWORD)parent_destroy_notes);
    if (result != 0x55 || IsWindow(temp))
        fail(197);
    SetLastError(SENTINEL);
    result = SendMessageW(temp, WM_USER, 0, 0);
    probe("send_destroyed_error", GetLastError());
    if (result != 0)
        fail(198);
}

/* The About box is a #32770 dialog owned by the main window on this thread. */
static BOOL CALLBACK find_box(HWND hwnd, LPARAM found)
{
    WCHAR klass[16];
    if (GetClassNameW(hwnd, klass, 16) == 6 && weq(klass, L"#32770") && GetWindow(hwnd, GW_OWNER) == main_hwnd) {
        *(HWND *)found = hwnd;
        return FALSE;
    }
    return TRUE;
}

static void CALLBACK about_timer(HWND hwnd, UINT message, UINT_PTR id, DWORD time)
{
    HWND box = NULL;
    WCHAR text[64];
    (void)hwnd;
    (void)time;
    if (message != WM_TIMER)
        return;
    if (InterlockedIncrement(&about_attempts) > 100) {
        KillTimer(NULL, id);
        return;
    }
    EnumThreadWindows(GetCurrentThreadId(), find_box, (LPARAM)&box);
    if (!box || !IsWindowVisible(box))
        return;
    KillTimer(NULL, id);
    if (GetWindowTextW(box, text, 64) == 18 && weq(text, L"About Doors Editor"))
        InterlockedOr(&about_checks, 1);
#ifdef EDITOR_RESOURCES
    if (GetDlgItemTextW(box, IDC_ABOUT_TEXT, text, 64) == 31 && weq(text, L"Doors Editor (resource variant)"))
        InterlockedOr(&about_checks, 2);
#else
    if (GetDlgItemTextW(box, 0xffff, text, 64) == 27 && weq(text, L"Doors Editor (code variant)"))
        InterlockedOr(&about_checks, 2);
#endif
    if (GetWindow(box, GW_OWNER) == main_hwnd)
        InterlockedOr(&about_checks, 4);
    /* Press the box's only button by its own control identifier. */
    {
        int button = GetDlgItem(box, IDOK) ? IDOK : IDCANCEL;
        about_button = button;
        PostMessageW(box, WM_COMMAND, MAKEWPARAM(button, BN_CLICKED), (LPARAM)GetDlgItem(box, button));
    }
}

static void about_dialog(void)
{
    INT_PTR result;
    if (!SetTimer(NULL, 0, 20, about_timer))
        fail(200);
    result = about();
    probe("about_button", (DWORD)about_button);
    probe("about_result", (DWORD)result);
    probe("about_checks", (DWORD)about_checks);
    if ((result != IDOK && result != about_button) || about_checks != 7 || !IsWindowEnabled(main_hwnd))
        fail(201);
}

static DWORD hit(int x, int y)
{
    return (DWORD)SendMessageW(chrome_hwnd, WM_NCHITTEST, 0, MAKELPARAM(x, y));
}

static void chrome(void)
{
    RECT client, window;
    SendMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_VIEW_CHROME, 0), 0);
    if (!chrome_hwnd || GetWindow(chrome_hwnd, GW_OWNER) != main_hwnd || !IsWindowVisible(chrome_hwnd))
        fail(210);
    GetClientRect(chrome_hwnd, &client);
    GetWindowRect(chrome_hwnd, &window);
    probe("chrome_client_w", (DWORD)client.right);
    probe("chrome_client_h", (DWORD)client.bottom);
    probe("chrome_window_x", (DWORD)window.left);
    probe("chrome_window_y", (DWORD)window.top);
    probe("chrome_window_w", (DWORD)(window.right - window.left));
    probe("chrome_window_h", (DWORD)(window.bottom - window.top));
    if (!GetClientRect(chrome_hwnd, &client) || !GetWindowRect(chrome_hwnd, &window) || client.left || client.top
        || client.right != CHROME_W || client.bottom != CHROME_H || window.left != CHROME_X || window.top != CHROME_Y
        || window.right != CHROME_X + CHROME_W || window.bottom != CHROME_Y + CHROME_H)
        fail(211);
    if (hit(CHROME_X + 50, CHROME_Y + 10) != HTCAPTION || hit(CHROME_X + CHROME_W - 20, CHROME_Y + 10) != HTCLIENT
        || hit(CHROME_X + CHROME_W - 8, CHROME_Y + CHROME_H - 8) != HTBOTTOMRIGHT
        || hit(CHROME_X + 100, CHROME_Y + 100) != HTCLIENT || hit(CHROME_X + CHROME_W + 40, CHROME_Y + 10) != HTNOWHERE)
        fail(212);
    if (!SetWindowPos(chrome_hwnd, NULL, CHROME_X + 40, CHROME_Y + 30, 0, 0,
                      SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
        || !GetWindowRect(chrome_hwnd, &window) || window.left != CHROME_X + 40 || window.top != CHROME_Y + 30)
        fail(213);
    SetWindowPos(chrome_hwnd, NULL, 0, 0, 100, 60, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    GetWindowRect(chrome_hwnd, &window);
    probe("chrome_small_w", (DWORD)(window.right - window.left));
    probe("chrome_small_h", (DWORD)(window.bottom - window.top));
    if (!SetWindowPos(chrome_hwnd, NULL, 0, 0, CHROME_W + 40, CHROME_H + 30, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE)
        || !GetClientRect(chrome_hwnd, &client) || client.right != CHROME_W + 40 || client.bottom != CHROME_H + 30)
        fail(214);
    SendMessageW(chrome_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(CHROME_W + 40 - 20, 16));
    SendMessageW(chrome_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(CHROME_W + 40 - 20, 16));
    if (chrome_hwnd)
        fail(215);
}

static void verify_saved(void)
{
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    DWORD expected = sizeof(doc_utf8) - 1, actual = 0, i;
    if (file == INVALID_HANDLE_VALUE || GetFileSize(file, NULL) != expected)
        fail(220);
    if (!ReadFile(file, file_buffer, FILE_MAX, &actual, NULL) || actual != expected || !CloseHandle(file))
        fail(221);
    for (i = 0; i < expected; ++i)
        if (file_buffer[i] != (BYTE)doc_utf8[i])
            fail(222);
}

static void close_main(UINT code)
{
    if (!PostMessageW(main_hwnd, WM_CLOSE, 0, 0))
        fail(code);
    {
        ULONGLONG deadline = GetTickCount64() + 5000;
        while (!quit_seen) {
            if (GetTickCount64() > deadline)
                fail(code + 1);
            pump_once(5);
        }
    }
    if (IsWindow(main_hwnd))
        fail(code + 2);
}

static __declspec(noreturn) void selftest(void)
{
    WCHAR title[96];
    create_main(SW_SHOWNOACTIVATE);
    check_menu();
    check_accelerators();
    check_strings();
    check_status();
    typing();
    reentrancy();
    about_dialog();
    chrome();
    SendMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_FILE_SAVE, 0), 0);
    if (!weq(file_part(path), doc_name) || SendMessageW(edit_hwnd, EM_GETMODIFY, 0, 0))
        fail(223);
    verify_saved();
    if (GetWindowTextW(main_hwnd, title, 96) != (int)wlen(doc_name) + 15 || !weq(title + wlen(doc_name), L" - Doors Editor"))
        fail(224);
    close_main(225);
    put("EDITOR_OK variant=" VARIANT " menu=1 accelerators=1 strings=1 edit=1 reentrant=1 about=1 chrome=1 save=1\r\n");
    flush_line();
    finish(EXIT_SELFTEST);
}

static __declspec(noreturn) void reopen(void)
{
    create_main(SW_SHOWNOACTIVATE);
    SendMessageW(main_hwnd, WM_COMMAND, MAKEWPARAM(ID_FILE_OPEN, 0), 0);
    if (!weq(file_part(path), doc_name))
        fail(230);
    if (!edit_is(doc_text) || SendMessageW(edit_hwnd, EM_GETMODIFY, 0, 0))
        fail(231);
    close_main(232);
    put("EDITOR_REOPEN variant=" VARIANT " dialog=1 text=1 chars=");
    put_dec(wlen(doc_text));
    put("\r\n");
    flush_line();
    finish(EXIT_REOPEN);
}

static __declspec(noreturn) void interactive(void)
{
    MSG message;
    int result;
    create_main(SW_SHOWNORMAL);
    SetForegroundWindow(main_hwnd);
    SetFocus(edit_hwnd);
    while ((result = GetMessageW(&message, NULL, 0, 0)) > 0) {
        if (!TranslateAcceleratorW(main_hwnd, accel, &message)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if (result < 0)
        fail(240);
    finish((UINT)message.wParam);
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
    out_handle = GetStdHandle(STD_OUTPUT_HANDLE);
    results_handle = CreateFileW(RESULTS_NAME, GENERIC_WRITE, FILE_SHARE_READ, NULL, OPEN_ALWAYS,
                                 FILE_ATTRIBUTE_NORMAL, NULL);
    if (results_handle == INVALID_HANDLE_VALUE || SetFilePointer(results_handle, 0, NULL, FILE_END) == INVALID_SET_FILE_POINTER)
        fail(91);
    if (!instance)
        fail(92);
    hooked = weq(mode, L"--selftest") || weq(mode, L"--reopen");
    if (mode[0] && !hooked)
        fail(93);
    put("EDITOR_START variant=" VARIANT " mode=");
    put(!mode[0] ? "interactive" : weq(mode, L"--selftest") ? "selftest" : "reopen");
    put("\r\n");
    flush_line();
    {
        INITCOMMONCONTROLSEX controls;
        controls.dwSize = sizeof(controls);
        controls.dwICC = ICC_BAR_CLASSES;
        if (!InitCommonControlsEx(&controls))
            fail(94);
    }
    register_classes();
    if (weq(mode, L"--selftest"))
        selftest();
    if (weq(mode, L"--reopen"))
        reopen();
    interactive();
}

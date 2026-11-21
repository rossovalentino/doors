/* INSTALL-A: Doors Tools' program (toolprobe.exe), an ordinary Windows
 * console program (Windows SDK, no C runtime) that the test packages
 * install and run:
 *
 *   toolprobe.exe mark <file>     writes "installed" (a deferred custom
 *                                 action's work outside the script)
 *   toolprobe.exe unmark <file>   deletes it (its rollback action)
 *   toolprobe.exe sleep <seconds> waits (INSTALL-A resets the machine here)
 *   toolprobe.exe open <file>     the .doorsnote file type's handler: adds
 *                                 a line to C:\ProgramData\Doors Tools
 *                                 Opened.txt
 *   toolprobe.exe                 prints "Doors Tools" and exits 0 */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>

static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
/* The command line's words (quotes group), into `words`. */
static int split(wchar_t *line, wchar_t **words, int capacity) {
    int count = 0;
    while (*line && count < capacity) {
        while (*line == L' ' || *line == L'\t') line++;
        if (!*line) break;
        wchar_t *start = line;
        wchar_t *out = line;
        int quoted = 0;
        while (*line && (quoted || (*line != L' ' && *line != L'\t'))) {
            if (*line == L'"') {
                quoted = !quoted;
                line++;
                continue;
            }
            *out++ = *line++;
        }
        if (*line) line++;
        *out = 0;
        words[count++] = start;
    }
    return count;
}
static int write_text(const wchar_t *path, const char *text, int append) {
    HANDLE h = CreateFileW(path, append ? FILE_APPEND_DATA : GENERIC_WRITE, FILE_SHARE_READ, NULL,
                           append ? OPEN_ALWAYS : CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return 0;
    DWORD n = 0, w = 0;
    while (text[n]) n++;
    BOOL ok = WriteFile(h, text, n, &w, NULL);
    CloseHandle(h);
    return ok && w == n;
}

void entry(void) {
    static wchar_t line[2048];
    const wchar_t *full = GetCommandLineW();
    int n = wlen(full);
    if (n >= 2047) n = 2047;
    for (int i = 0; i < n; i++) line[i] = full[i];
    line[n] = 0;
    wchar_t *words[8];
    int count = split(line, words, 8);
    UINT code = 0;
    if (count >= 3 && same(words[1], L"mark")) {
        code = write_text(words[2], "installed", 0) ? 0 : 2;
    } else if (count >= 3 && same(words[1], L"unmark")) {
        DeleteFileW(words[2]);
    } else if (count >= 3 && same(words[1], L"sleep")) {
        DWORD seconds = 0;
        for (const wchar_t *p = words[2]; *p >= L'0' && *p <= L'9'; p++) seconds = seconds * 10 + (DWORD)(*p - L'0');
        Sleep(seconds * 1000);
    } else if (count >= 3 && same(words[1], L"open")) {
        static char text[1200];
        int at = 0;
        const char *head = "opened ";
        while (*head) text[at++] = *head++;
        for (const wchar_t *p = words[2]; *p && at < 1100; p++) text[at++] = (char)(*p < 128 ? *p : '?');
        text[at++] = '\r';
        text[at++] = '\n';
        text[at] = 0;
        code = write_text(L"C:\\ProgramData\\Doors Tools Opened.txt", text, 1) ? 0 : 3;
    } else {
        DWORD w = 0;
        WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), "Doors Tools\r\n", 13, &w, NULL);
    }
    ExitProcess(code);
}

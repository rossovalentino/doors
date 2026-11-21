/* CONNECT-A WinINet probe (P15): an ordinary Windows console program
 * (Windows SDK, no C runtime) that uses wininet.dll, ANSI and Unicode,
 * against the test web server the acceptance run starts on this host:
 *
 *   inetprobe.exe <port> [server address, default 10.0.2.2] [ftp port]
 *
 * One "INET-A <step> ..." line per check; exit 0 only when all passed.
 * The same binary runs on Windows against the same server as reference. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <wininet.h>
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
static int same(const char *a, const char *b, int n) {
    for (int i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}
static int equal(const char *a, const char *b) {
    return length(a) == length(b) && same(a, b, length(a));
}
static int ends(const char *text, const char *tail) {
    int n = length(text), m = length(tail);
    return n >= m && same(text + n - m, tail, m);
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("INET-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static void note(const char *name, long value) {
    put("INET-A note ");
    put(name);
    put("=");
    number(value);
    put("\r\n");
}
static void append(char *to, const char *from) {
    int n = length(to);
    for (int i = 0; from[i]; i++) to[n++] = from[i];
    to[n] = 0;
}
static void append_number(char *to, unsigned v) {
    char b[12];
    int i = 11;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    append(to, b + i);
}

/* Reads a whole response body (InternetReadFile fills the buffer). */
static int read_all(HINTERNET request, char *buffer, int capacity) {
    int got = 0;
    for (;;) {
        DWORD n = 0;
        if (!InternetReadFile(request, buffer + got, (DWORD)(capacity - got), &n) || n == 0) break;
        got += (int)n;
        if (got == capacity) break;
    }
    return got;
}
static DWORD status_of(HINTERNET request) {
    DWORD status = 0, size = sizeof status;
    HttpQueryInfoA(request, HTTP_QUERY_STATUS_CODE | HTTP_QUERY_FLAG_NUMBER, &status, &size, NULL);
    return status;
}
/* GET `path` with credentials given to InternetConnect: the body read and
 * the final status in *status. */
static int authenticated(HINTERNET internet, const wchar_t *host, INTERNET_PORT port, const wchar_t *path,
                         const wchar_t *user, char *buffer, int capacity, DWORD *status) {
    HINTERNET connect = InternetConnectW(internet, host, port, user, L"secret", INTERNET_SERVICE_HTTP, 0, 0);
    HINTERNET request = connect ? HttpOpenRequestW(connect, L"GET", path, NULL, NULL, NULL,
                                                   INTERNET_FLAG_RELOAD | INTERNET_FLAG_KEEP_CONNECTION, 0)
                                : NULL;
    int got = 0;
    *status = 0;
    if (request && HttpSendRequestW(request, NULL, 0, NULL, 0)) {
        *status = status_of(request);
        got = read_all(request, buffer, capacity);
    }
    if (request) InternetCloseHandle(request);
    if (connect) InternetCloseHandle(connect);
    return got;
}

static HANDLE completed;
static volatile DWORD_PTR async_result;
static volatile DWORD async_error, completions;
static void CALLBACK callback(HINTERNET handle, DWORD_PTR context, DWORD status, LPVOID info, DWORD info_length) {
    (void)handle;
    (void)context;
    (void)info_length;
    if (status == INTERNET_STATUS_REQUEST_COMPLETE) {
        INTERNET_ASYNC_RESULT *r = (INTERNET_ASYNC_RESULT *)info;
        async_result = r->dwResult;
        async_error = r->dwError;
        completions++;
        SetEvent(completed);
    }
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    char host[32] = "10.0.2.2";
    unsigned port = 0, ftp_port = 0;
    {
        LPSTR line = GetCommandLineA();
        while (*line && *line != ' ') line++;
        while (*line == ' ') line++;
        while (*line >= '0' && *line <= '9') port = port * 10 + (unsigned)(*line++ - '0');
        while (*line == ' ') line++;
        if (*line) {
            int i = 0;
            while (line[i] && line[i] != ' ' && i < 31) {
                host[i] = line[i];
                i++;
            }
            host[i] = 0;
            line += i;
            while (*line == ' ') line++;
            /* The FTP server's port (step 14), when given. */
            while (*line >= '0' && *line <= '9') ftp_port = ftp_port * 10 + (unsigned)(*line++ - '0');
        }
    }
    if (!port) {
        put("usage: inetprobe <port> [server]\r\n");
        ExitProcess(100);
    }
    char base[64] = "http://";
    append(base, host);
    append(base, ":");
    append_number(base, port);
    static char body[256], url[128], text[128];
    HINTERNET internet = InternetOpenA("Doors-InetProbe/1.0", INTERNET_OPEN_TYPE_DIRECT, NULL, NULL, 0);

    /* 1. InternetOpenUrl (ANSI): status and body. */
    url[0] = 0;
    append(url, base);
    append(url, "/hello");
    HINTERNET request = InternetOpenUrlA(internet, url, NULL, 0, INTERNET_FLAG_RELOAD, 0);
    DWORD status = request ? status_of(request) : 0;
    int got = request ? read_all(request, body, sizeof body) : 0;
    DWORD size = sizeof text;
    int typed = request && HttpQueryInfoA(request, HTTP_QUERY_CONTENT_TYPE, text, &size, NULL) && equal(text, "text/plain") && size == 10;
    if (request) InternetCloseHandle(request);
    step(1, "open-url", status == 200 && got == 13 && same(body, "hello winhttp", 13) && typed, (long)status);

    /* 2. Connect, open, send (Unicode): a chunked body. */
    wchar_t whost[32];
    for (int i = 0; i < 32; i++) whost[i] = (wchar_t)host[i];
    HINTERNET connect = InternetConnectW(internet, whost, (INTERNET_PORT)port, NULL, NULL, INTERNET_SERVICE_HTTP, 0, 0);
    request = HttpOpenRequestW(connect, L"GET", L"/chunked", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    int sent = HttpSendRequestW(request, NULL, 0, NULL, 0);
    got = sent ? read_all(request, body, sizeof body) : 0;
    InternetCloseHandle(request);
    step(2, "chunked", sent && got == 19 && same(body, "chunked bodies work", 19), got);

    /* 3. A followed redirect; the URL option is the target. */
    request = HttpOpenRequestA(connect, "GET", "/redirect", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    sent = HttpSendRequestA(request, NULL, 0, NULL, 0);
    status = sent ? status_of(request) : 0;
    size = sizeof text;
    int moved = sent && InternetQueryOptionA(request, INTERNET_OPTION_URL, text, &size) && ends(text, "/hello");
    got = sent ? read_all(request, body, sizeof body) : 0;
    InternetCloseHandle(request);
    step(3, "redirect", status == 200 && moved && got == 13, (long)status);

    /* 4. POST with an added header; a custom header query names the
     * header in the buffer. */
    request = HttpOpenRequestA(connect, "POST", "/echo", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    int added = HttpAddRequestHeadersA(request, "X-Doors: 26H2\r\n", (DWORD)-1L, HTTP_ADDREQ_FLAG_ADD);
    sent = added && HttpSendRequestA(request, NULL, 0, "posted bytes", 12);
    char custom[64] = "X-Echo-Doors";
    size = sizeof custom;
    int reflected = sent && HttpQueryInfoA(request, HTTP_QUERY_CUSTOM, custom, &size, NULL) && equal(custom, "26H2");
    got = sent ? read_all(request, body, sizeof body) : 0;
    InternetCloseHandle(request);
    step(4, "post", reflected && got == 12 && same(body, "posted bytes", 12), got);

    /* 5. Cookies: stored from a response, read back by URL, and sent. */
    request = HttpOpenRequestA(connect, "GET", "/set-cookie", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    HttpSendRequestA(request, NULL, 0, NULL, 0);
    read_all(request, body, sizeof body);
    InternetCloseHandle(request);
    char cookie[64];
    DWORD cookie_size = sizeof cookie;
    url[0] = 0;
    append(url, base);
    append(url, "/");
    int stored = InternetGetCookieA(url, NULL, cookie, &cookie_size) && equal(cookie, "session=abc");
    request = HttpOpenRequestA(connect, "GET", "/cookie", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    sent = HttpSendRequestA(request, NULL, 0, NULL, 0);
    got = sent ? read_all(request, body, sizeof body) : 0;
    InternetCloseHandle(request);
    step(5, "cookies", stored && got == 11 && same(body, "session=abc", 11), got);

    /* 6. 404 with its status text. */
    request = HttpOpenRequestA(connect, "GET", "/nowhere", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    sent = HttpSendRequestA(request, NULL, 0, NULL, 0);
    size = sizeof text;
    int reason = sent && HttpQueryInfoA(request, HTTP_QUERY_STATUS_TEXT, text, &size, NULL) && equal(text, "Not Found");
    status = sent ? status_of(request) : 0;
    InternetCloseHandle(request);
    InternetCloseHandle(connect);
    step(6, "not-found", status == 404 && reason, (long)status);

    /* 7. URL helpers. */
    URL_COMPONENTSA parts;
    memset(&parts, 0, sizeof parts);
    parts.dwStructSize = sizeof parts;
    parts.dwHostNameLength = parts.dwUrlPathLength = parts.dwExtraInfoLength = (DWORD)-1;
    char canonical[128], combined[128];
    DWORD canonical_length = sizeof canonical, combined_length = sizeof combined;
    int urls = InternetCrackUrlA("https://Example.COM:8443/a/b?c=d#frag", 0, 0, &parts) && parts.nScheme == INTERNET_SCHEME_HTTPS &&
        parts.nPort == 8443 && parts.dwHostNameLength == 11 && parts.dwUrlPathLength == 4 && parts.dwExtraInfoLength == 9 &&
        InternetCanonicalizeUrlA("http://h/a/./b/../c d?x y", canonical, &canonical_length, 0) && equal(canonical, "http://h/a/c%20d?x y") &&
        InternetCombineUrlA("http://a/b/c/d;p?q", "../g", combined, &combined_length, 0) && equal(combined, "http://a/b/g");
    step(7, "urls", urls, (long)canonical_length);

    /* 8. Connected state. */
    DWORD connected_flags = 0;
    int connected = InternetGetConnectedState(&connected_flags, 0);
    step(8, "connected", connected && (connected_flags & INTERNET_CONNECTION_LAN), (long)connected_flags);

    /* 9. Asynchronous InternetOpenUrl and reads. */
    completed = CreateEventA(NULL, FALSE, FALSE, NULL);
    HINTERNET async_internet = InternetOpenA("Doors-InetProbe/1.0", INTERNET_OPEN_TYPE_DIRECT, NULL, NULL, INTERNET_FLAG_ASYNC);
    InternetSetStatusCallbackA(async_internet, callback);
    url[0] = 0;
    append(url, base);
    append(url, "/hello");
    HINTERNET async_request = InternetOpenUrlA(async_internet, url, NULL, 0, INTERNET_FLAG_RELOAD, 1);
    DWORD pending = async_request ? 0 : GetLastError();
    if (!async_request && pending == ERROR_IO_PENDING && WaitForSingleObject(completed, 15000) == WAIT_OBJECT_0 && async_error == 0) {
        async_request = (HINTERNET)async_result;
    }
    int async_got = 0;
    for (int i = 0; async_request && i < 16; i++) {
        DWORD n = 0;
        if (!InternetReadFile(async_request, body + async_got, (DWORD)(sizeof body - async_got), &n)) {
            if (GetLastError() != ERROR_IO_PENDING || WaitForSingleObject(completed, 15000) != WAIT_OBJECT_0 || async_error) break;
        }
        if (n == 0) break;
        async_got += (int)n;
    }
    if (async_request) InternetCloseHandle(async_request);
    InternetCloseHandle(async_internet);
    step(9, "async", pending == ERROR_IO_PENDING && async_got == 13 && same(body, "hello winhttp", 13), async_got);

    /* 10. TLS against the plain-HTTP port. */
    url[0] = 0;
    append(url, "https://");
    append(url, host);
    append(url, ":");
    append_number(url, port);
    append(url, "/hello");
    request = InternetOpenUrlA(internet, url, NULL, 0, INTERNET_FLAG_RELOAD, 0);
    DWORD secure_error = request ? 0 : GetLastError();
    if (request) InternetCloseHandle(request);
    step(10, "secure-failure", !request && secure_error == ERROR_INTERNET_SECURITY_CHANNEL_ERROR, (long)secure_error);

    /* 11. INTERNET_OPTION_HTTP_DECODING: the request asks for gzip itself
     *     and WinINet hands back the decoded body. */
    HINTERNET decoder = InternetConnectW(internet, whost, (INTERNET_PORT)port, NULL, NULL, INTERNET_SERVICE_HTTP, 0, 0);
    request = HttpOpenRequestW(decoder, L"GET", L"/gzip", NULL, NULL, NULL, INTERNET_FLAG_RELOAD, 0);
    BOOL decode = TRUE;
    int decoding = request && InternetSetOptionW(request, INTERNET_OPTION_HTTP_DECODING, &decode, sizeof decode);
    sent = decoding && HttpSendRequestW(request, L"Accept-Encoding: gzip\r\n", (DWORD)-1L, NULL, 0);
    got = sent ? read_all(request, body, sizeof body) : 0;
    InternetCloseHandle(request);
    InternetCloseHandle(decoder);
    step(11, "decoding", sent && got == 26 && same(body, "decompressed by the client", 26), got);

    /* 12. Credentials given to InternetConnect answer a Digest challenge
     *     inside HttpSendRequest. (NTLM is not checked here: on the
     *     reference PC WinINet first offers the signed-in Microsoft
     *     account to a loopback server and stops at the 401; WinHTTP's
     *     steps cover the shared NTLM engine.) */
    DWORD auth_status = 0;
    got = authenticated(internet, whost, (INTERNET_PORT)port, L"/digest", L"doors", body, sizeof body, &auth_status);
    step(12, "digest-auth", auth_status == 200 && got == 9 && same(body, "digest ok", 9), (long)auth_status);

    /* 13. The URL cache: a cacheable response is stored as it is read, its
     *     entry names a local file with the body, INTERNET_FLAG_FROM_CACHE
     *     reads it back without the network; an entry made by hand
     *     (CreateUrlCacheEntry, CommitUrlCacheEntry) is enumerated; both
     *     are deleted again. */
    {
        static char cached_url[128];
        static wchar_t wcached[128], wfile[MAX_PATH];
        static BYTE info_buffer[4096];
        cached_url[0] = 0;
        append(cached_url, base);
        append(cached_url, "/cached");
        {
            int i = 0;
            for (; cached_url[i]; i++) wcached[i] = (wchar_t)cached_url[i];
            wcached[i] = 0;
        }
        HINTERNET fetched = InternetOpenUrlA(internet, cached_url, NULL, 0, 0, 0);
        int fetched_bytes = fetched ? read_all(fetched, body, sizeof body) : 0;
        if (fetched) InternetCloseHandle(fetched);
        DWORD info_size = 0;
        int sized = !GetUrlCacheEntryInfoW(wcached, NULL, &info_size) && GetLastError() == ERROR_INSUFFICIENT_BUFFER &&
                    info_size > sizeof(INTERNET_CACHE_ENTRY_INFOW);
        INTERNET_CACHE_ENTRY_INFOW *info = (INTERNET_CACHE_ENTRY_INFOW *)info_buffer;
        info_size = sizeof info_buffer;
        int found = GetUrlCacheEntryInfoW(wcached, info, &info_size);
        int file_ok = 0;
        if (found && info->lpszLocalFileName) {
            HANDLE f = CreateFileW(info->lpszLocalFileName, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE, NULL,
                                   OPEN_EXISTING, 0, NULL);
            char content[64];
            DWORD n = 0;
            if (f != INVALID_HANDLE_VALUE) {
                file_ok = ReadFile(f, content, sizeof content, &n, NULL) && n == 17 && same(content, "cached by wininet", 17);
                CloseHandle(f);
            }
        }
        note("cache_struct_size", (long)sizeof(INTERNET_CACHE_ENTRY_INFOW));
        note("cache_entry_type", found ? (long)info->CacheEntryType : -1);
        note("cache_size", found ? (long)info->dwSizeLow : -1);
        note("cache_header_size", found ? (long)info->dwHeaderInfoSize : -1);
        HINTERNET again = InternetOpenUrlA(internet, cached_url, NULL, 0, INTERNET_FLAG_FROM_CACHE, 0);
        int from_cache = again ? read_all(again, body, sizeof body) : 0;
        if (again) InternetCloseHandle(again);
        int deleted = DeleteUrlCacheEntryW(wcached);
        int gone = !GetUrlCacheEntryInfoW(wcached, NULL, &info_size) && GetLastError() == ERROR_FILE_NOT_FOUND;
        HINTERNET missing = InternetOpenUrlA(internet, cached_url, NULL, 0, INTERNET_FLAG_FROM_CACHE, 0);
        DWORD missing_error = missing ? 0 : GetLastError();
        if (missing) InternetCloseHandle(missing);
        note("from_cache_missing", (long)missing_error);
        /* By hand. */
        static const wchar_t manual[] = L"http://doors.test/manual-entry.txt";
        int created = CreateUrlCacheEntryW(manual, 6, L"txt", wfile, 0);
        int written = 0;
        if (created) {
            HANDLE f = CreateFileW(wfile, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
            DWORD n = 0;
            written = f != INVALID_HANDLE_VALUE && WriteFile(f, "manual", 6, &n, NULL) && n == 6;
            if (f != INVALID_HANDLE_VALUE) CloseHandle(f);
        }
        FILETIME none = {0, 0};
        static const wchar_t head[] = L"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
        int committed = written && CommitUrlCacheEntryW(manual, wfile, none, none, NORMAL_CACHE_ENTRY, (LPWSTR)head,
                                                        (DWORD)(sizeof head / sizeof head[0] - 1), L"txt", NULL);
        int listed = 0;
        info_size = sizeof info_buffer;
        HANDLE find = FindFirstUrlCacheEntryW(NULL, info, &info_size);
        for (int guard = 0; find && guard < 100000 && !listed; guard++) {
            const wchar_t *u = info->lpszSourceUrlName;
            int match = 1;
            for (int i = 0; match && (manual[i] || u[i]); i++) match = manual[i] == u[i];
            if (match) {
                listed = info->dwSizeLow == 6;
                break;
            }
            info_size = sizeof info_buffer;
            if (!FindNextUrlCacheEntryW(find, info, &info_size)) break;
        }
        if (find) FindCloseUrlCache(find);
        int manual_deleted = DeleteUrlCacheEntryW(manual);
        note("cache_steps", (fetched_bytes == 17) | sized << 1 | found << 2 | file_ok << 3 | (from_cache == 17) << 4 |
                                deleted << 5 | gone << 6 | created << 7 | committed << 8 | listed << 9 |
                                manual_deleted << 10);
        step(13, "url-cache",
             fetched_bytes == 17 && sized && found && file_ok && from_cache == 17 && deleted && gone && created &&
                 committed && listed && manual_deleted,
             (long)info_size);
    }

    /* 14. FTP (passive) to the test server: the current directory, a
     *     listing, a file read, a file written, renamed and deleted, a
     *     directory made, entered and removed, and an ftp:// URL. */
    if (ftp_port) {
        HINTERNET ftp = InternetConnectA(internet, host, (INTERNET_PORT)ftp_port, "anonymous", "doors@example.test",
                                         INTERNET_SERVICE_FTP, INTERNET_FLAG_PASSIVE, 0);
        DWORD ftp_error = ftp ? 0 : GetLastError();
        wchar_t directory[MAX_PATH];
        DWORD directory_length = MAX_PATH;
        int pwd_called = ftp && FtpGetCurrentDirectoryW(ftp, directory, &directory_length);
        note("ftp_pwd_length", (long)directory_length);
        // The length counts the terminating NUL, as measured on Windows.
        int pwd = pwd_called && directory_length == 2 &&
                  directory[0] == L'/';
        WIN32_FIND_DATAW found_file;
        int names = 0;
        HINTERNET find = ftp ? FtpFindFirstFileW(ftp, L"*", &found_file, INTERNET_FLAG_RELOAD, 0) : NULL;
        if (find) {
            do {
                const wchar_t *n = found_file.cFileName;
                if (n[0] == L'h' && n[1] == L'e' && n[5] == L'.' && !(found_file.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) &&
                    found_file.nFileSizeLow == 14)
                    names |= 1;
                if (n[0] == L'p' && n[1] == L'u' && n[2] == L'b' && !n[3] &&
                    (found_file.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY))
                    names |= 2;
            } while (InternetFindNextFileW(find, &found_file));
            note("ftp_find_end", (long)GetLastError());
            InternetCloseHandle(find);
        }
        HINTERNET file = ftp ? FtpOpenFileW(ftp, L"hello.txt", GENERIC_READ, FTP_TRANSFER_TYPE_BINARY, 0) : NULL;
        int read = file ? read_all(file, body, sizeof body) : 0;
        if (file) InternetCloseHandle(file);
        HINTERNET put_file = ftp ? FtpOpenFileW(ftp, L"upload.txt", GENERIC_WRITE, FTP_TRANSFER_TYPE_BINARY, 0) : NULL;
        DWORD put_bytes = 0;
        int uploaded = put_file && InternetWriteFile(put_file, "uploaded!", 9, &put_bytes) && put_bytes == 9;
        if (put_file) InternetCloseHandle(put_file);
        DWORD high = 0;
        HINTERNET sized_file = ftp ? FtpOpenFileW(ftp, L"upload.txt", GENERIC_READ, FTP_TRANSFER_TYPE_BINARY, 0) : NULL;
        DWORD low = sized_file ? FtpGetFileSize(sized_file, &high) : 0;
        if (sized_file) InternetCloseHandle(sized_file);
        int renamed = ftp && FtpRenameFileW(ftp, L"upload.txt", L"renamed.txt");
        int removed = ftp && FtpDeleteFileW(ftp, L"renamed.txt");
        int missing_delete = ftp && !FtpDeleteFileW(ftp, L"renamed.txt");
        DWORD missing_error = GetLastError();
        int made = ftp && FtpCreateDirectoryW(ftp, L"made");
        directory_length = MAX_PATH;
        int entered_called = made && FtpSetCurrentDirectoryW(ftp, L"made") &&
                             FtpGetCurrentDirectoryW(ftp, directory, &directory_length);
        note("ftp_made_length", (long)directory_length);
        int entered = entered_called && directory_length == 6;
        int left = entered && FtpSetCurrentDirectoryW(ftp, L"..");
        int unmade = made && FtpRemoveDirectoryW(ftp, L"made");
        if (ftp) InternetCloseHandle(ftp);
        char ftp_url[96] = "ftp://";
        append(ftp_url, host);
        append(ftp_url, ":");
        append_number(ftp_url, ftp_port);
        append(ftp_url, "/hello.txt");
        HINTERNET by_url = InternetOpenUrlA(internet, ftp_url, NULL, 0, INTERNET_FLAG_PASSIVE | INTERNET_FLAG_RELOAD, 0);
        int url_read = by_url ? read_all(by_url, body + 64, 64) : 0;
        if (by_url) InternetCloseHandle(by_url);
        note("ftp_connect_error", (long)ftp_error);
        note("ftp_missing_delete", (long)missing_error);
        note("ftp_steps", pwd | names << 1 | (read == 14) << 3 | uploaded << 4 | (low == 9) << 5 | renamed << 6 |
                              removed << 7 | missing_delete << 8 | made << 9 | entered << 10 | left << 11 |
                              unmade << 12 | (url_read == 14) << 13);
        step(14, "ftp",
             pwd && names == 3 && read == 14 && same(body, "hello over ftp", 14) && uploaded && low == 9 && renamed &&
                 removed && missing_delete && missing_error == ERROR_INTERNET_EXTENDED_ERROR && made && entered &&
                 left && unmade && url_read == 14,
             (long)missing_error);
    }

    InternetCloseHandle(internet);
    put(failed_step ? "INET-A FAILED\r\n" : "INET-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

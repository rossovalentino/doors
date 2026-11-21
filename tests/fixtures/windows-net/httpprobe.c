/* CONNECT-A WinHTTP probe (P15): an ordinary Windows console program
 * (Windows SDK, no C runtime) that uses winhttp.dll against the test web
 * server the acceptance run starts on this host:
 *
 *   httpprobe.exe <port> <server address> <tls port> <wrong-host port> <expired port>
 *
 * The three TLS ports are the acceptance run's HTTPS endpoints (TLS 1.3,
 * self-signed P-256 certificates: valid, for another host, expired).
 * One "HTTP-A <step> ..." line per check; exit 0 only when all passed.
 * The same binary runs on Windows against the same server as reference. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <winhttp.h>
#include <intrin.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    __stosb((unsigned char *)destination, (unsigned char)value, count);
    return destination;
}
#pragma function(memcpy)
void *memcpy(void *destination, const void *source, size_t count) {
    __movsb((unsigned char *)destination, (const unsigned char *)source, count);
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
static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static int wide_ends(const wchar_t *text, const wchar_t *tail) {
    int n = 0, m = 0;
    while (text[n]) n++;
    while (tail[m]) m++;
    return n >= m && wide_equal(text + n - m, tail);
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("HTTP-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static void note(const char *name, long value) {
    put("HTTP-A note ");
    put(name);
    put("=");
    number(value);
    put("\r\n");
}

static wchar_t server[32];
static INTERNET_PORT port, tls_good, tls_wrong_host, tls_expired;
static volatile LONG redirects;
static void CALLBACK watch(HINTERNET handle, DWORD_PTR context, DWORD status, LPVOID info, DWORD info_length) {
    (void)handle;
    (void)context;
    (void)info;
    (void)info_length;
    if (status == WINHTTP_CALLBACK_STATUS_REDIRECT) InterlockedIncrement(&redirects);
}

/* One synchronous request; the body (at most `capacity` bytes) and the
 * status code. */
static DWORD fetch(HINTERNET session, const wchar_t *verb, const wchar_t *path, const wchar_t *headers, const char *body,
                   char *response, int capacity, int *received, HINTERNET *keep) {
    HINTERNET connect = WinHttpConnect(session, server, port, 0);
    HINTERNET request = connect ? WinHttpOpenRequest(connect, verb, path, NULL, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, 0) : NULL;
    DWORD status = 0, size = sizeof status;
    *received = 0;
    if (!request) return 0;
    if (headers) WinHttpAddRequestHeaders(request, headers, (DWORD)-1L, WINHTTP_ADDREQ_FLAG_ADD);
    DWORD body_length = body ? (DWORD)length(body) : 0;
    if (WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0, (LPVOID)body, body_length, body_length, 0) &&
        WinHttpReceiveResponse(request, NULL) &&
        WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX, &status, &size, WINHTTP_NO_HEADER_INDEX)) {
        DWORD available = 0;
        while (WinHttpQueryDataAvailable(request, &available) && available) {
            DWORD got = 0;
            if (*received + (int)available > capacity) available = (DWORD)(capacity - *received);
            if (!available || !WinHttpReadData(request, response + *received, available, &got) || !got) break;
            *received += (int)got;
        }
    }
    if (keep) {
        *keep = request;
    } else {
        WinHttpCloseHandle(request);
    }
    WinHttpCloseHandle(connect);
    return status;
}

/* GET `path`: its 401 must offer `scheme`; the second send carries the
 * credentials for it. The final status (0 on any failure) and the body. */
static DWORD authenticated(HINTERNET session, const wchar_t *path, DWORD scheme, const wchar_t *user, char *response,
                           int capacity, int *received) {
    HINTERNET connect = WinHttpConnect(session, server, port, 0);
    HINTERNET request = connect ? WinHttpOpenRequest(connect, L"GET", path, NULL, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, 0) : NULL;
    DWORD status = 0, size = sizeof status, supported = 0, first = 0, target = 0;
    *received = 0;
    int ok = request && WinHttpSendRequest(request, NULL, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL) &&
             WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, NULL, &status, &size, NULL) &&
             status == 401 && WinHttpQueryAuthSchemes(request, &supported, &first, &target) && (supported & scheme) &&
             target == WINHTTP_AUTH_TARGET_SERVER;
    /* Read the refusal's body so the connection can carry the answer. */
    DWORD available = 0, got = 0;
    while (ok && WinHttpQueryDataAvailable(request, &available) && available) {
        if (available > (DWORD)capacity) available = (DWORD)capacity;
        if (!WinHttpReadData(request, response, available, &got) || !got) break;
    }
    status = 0;
    size = sizeof status;
    ok = ok && WinHttpSetCredentials(request, WINHTTP_AUTH_TARGET_SERVER, scheme, user, L"secret", NULL) &&
         WinHttpSendRequest(request, NULL, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL) &&
         WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, NULL, &status, &size, NULL);
    while (ok && WinHttpQueryDataAvailable(request, &available) && available) {
        if (*received + (int)available > capacity) available = (DWORD)(capacity - *received);
        if (!available || !WinHttpReadData(request, response + *received, available, &got) || !got) break;
        *received += (int)got;
    }
    if (request) WinHttpCloseHandle(request);
    if (connect) WinHttpCloseHandle(connect);
    return ok ? status : 0;
}

/* HTTPS: the secure failure's reasons (WINHTTP_CALLBACK_STATUS_FLAG_*). */
static volatile LONG secure_reasons;
static void CALLBACK secure_watch(HINTERNET handle, DWORD_PTR context, DWORD status, LPVOID info, DWORD info_length) {
    (void)handle;
    (void)context;
    (void)info_length;
    if (status == WINHTTP_CALLBACK_STATUS_SECURE_FAILURE && info) secure_reasons = (LONG)*(DWORD *)info;
}
/* GET https://server:tls_port/hello with the security flags `ignore`, in
 * a session of its own (WinHTTP keeps a certificate accepted once for the
 * session's later requests to that server): the status (0 when it
 * failed), the error, the body and the security flags the request reports
 * afterwards. */
static DWORD secure_get(INTERNET_PORT tls_port, DWORD ignore, char *response, int capacity, int *received, DWORD *error,
                        DWORD *flags_after) {
    HINTERNET session = WinHttpOpen(L"Doors-HttpProbe/1.0", WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    HINTERNET connect = session ? WinHttpConnect(session, server, tls_port, 0) : NULL;
    HINTERNET request = connect ? WinHttpOpenRequest(connect, L"GET", L"/hello", NULL, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, WINHTTP_FLAG_SECURE) : NULL;
    DWORD status = 0, size = sizeof status;
    *received = 0;
    *error = 0;
    *flags_after = 0;
    secure_reasons = 0;
    if (request) {
        WinHttpSetStatusCallback(request, secure_watch, WINHTTP_CALLBACK_FLAG_SECURE_FAILURE, 0);
        if (ignore) WinHttpSetOption(request, WINHTTP_OPTION_SECURITY_FLAGS, &ignore, sizeof ignore);
    }
    if (request && WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL) &&
        WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX, &status, &size, WINHTTP_NO_HEADER_INDEX)) {
        DWORD available = 0, flags = 0, flags_size = sizeof flags;
        while (WinHttpQueryDataAvailable(request, &available) && available) {
            DWORD got = 0;
            if (*received + (int)available > capacity) available = (DWORD)(capacity - *received);
            if (!available || !WinHttpReadData(request, response + *received, available, &got) || !got) break;
            *received += (int)got;
        }
        if (WinHttpQueryOption(request, WINHTTP_OPTION_SECURITY_FLAGS, &flags, &flags_size)) *flags_after = flags;
    } else {
        *error = GetLastError();
        status = 0;
    }
    if (request) WinHttpCloseHandle(request);
    if (connect) WinHttpCloseHandle(connect);
    if (session) WinHttpCloseHandle(session);
    return status;
}

/* Asynchronous GET /hello: completion events drive the reads. */
static HANDLE done_event;
static char async_body[64];
static volatile LONG async_received, async_error;
static void CALLBACK async_callback(HINTERNET request, DWORD_PTR context, DWORD status, LPVOID info, DWORD info_length) {
    (void)context;
    switch (status) {
    case WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE:
        if (!WinHttpReceiveResponse(request, NULL)) SetEvent(done_event);
        break;
    case WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE:
        if (!WinHttpQueryDataAvailable(request, NULL)) SetEvent(done_event);
        break;
    case WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE:
        if (*(DWORD *)info == 0) {
            SetEvent(done_event);
        } else if (!WinHttpReadData(request, async_body + async_received, sizeof async_body - async_received - 1, NULL)) {
            SetEvent(done_event);
        }
        break;
    case WINHTTP_CALLBACK_STATUS_READ_COMPLETE:
        async_received += (LONG)info_length;
        if (info_length == 0 || !WinHttpQueryDataAvailable(request, NULL)) SetEvent(done_event);
        break;
    case WINHTTP_CALLBACK_STATUS_REQUEST_ERROR:
        async_error = (LONG)((WINHTTP_ASYNC_RESULT *)info)->dwError;
        SetEvent(done_event);
        break;
    }
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    {
        LPWSTR line = GetCommandLineW();
        INTERNET_PORT *ports[4] = {&port, &tls_good, &tls_wrong_host, &tls_expired};
        while (*line && *line != ' ') line++;
        while (*line == ' ') line++;
        while (*line >= '0' && *line <= '9') port = (INTERNET_PORT)(port * 10 + (*line++ - '0'));
        while (*line == ' ') line++;
        for (int i = 0; *line && *line != ' ' && i < 31; i++) server[i] = *line++;
        for (int p = 1; p < 4; p++) {
            while (*line == ' ') line++;
            while (*line >= '0' && *line <= '9') *ports[p] = (INTERNET_PORT)(*ports[p] * 10 + (*line++ - '0'));
        }
    }
    if (!port || !server[0] || !tls_good || !tls_wrong_host || !tls_expired) {
        put("usage: httpprobe <port> <server> <tls port> <wrong-host port> <expired port>\r\n");
        ExitProcess(100);
    }
    static char body[256];
    int got = 0;
    HINTERNET session = WinHttpOpen(L"Doors-HttpProbe/1.0", WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    WinHttpSetStatusCallback(session, watch, WINHTTP_CALLBACK_FLAG_REDIRECT, 0);

    /* 1. GET with Content-Length; headers by number and by name. */
    HINTERNET request = NULL;
    DWORD status = fetch(session, L"GET", L"/hello", NULL, NULL, body, sizeof body, &got, &request);
    wchar_t text[64];
    DWORD size = sizeof text;
    DWORD length_value = 0, length_size = sizeof length_value;
    int headers_ok = request && WinHttpQueryHeaders(request, WINHTTP_QUERY_CONTENT_TYPE, WINHTTP_HEADER_NAME_BY_INDEX, text, &size, WINHTTP_NO_HEADER_INDEX) &&
        wide_equal(text, L"text/plain") && size == 20 &&
        WinHttpQueryHeaders(request, WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX, &length_value, &length_size, WINHTTP_NO_HEADER_INDEX) &&
        length_value == 13;
    size = sizeof text;
    int text_ok = request && WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_TEXT, WINHTTP_HEADER_NAME_BY_INDEX, text, &size, WINHTTP_NO_HEADER_INDEX) && wide_equal(text, L"OK");
    if (request) WinHttpCloseHandle(request);
    step(1, "get", status == 200 && got == 13 && same(body, "hello winhttp", 13) && headers_ok && text_ok, (long)status);

    /* 2. A chunked body. */
    status = fetch(session, L"GET", L"/chunked", NULL, NULL, body, sizeof body, &got, NULL);
    step(2, "chunked", status == 200 && got == 19 && same(body, "chunked bodies work", 19), got);

    /* 3. A redirect followed automatically; the URL option is the target. */
    request = NULL;
    status = fetch(session, L"GET", L"/redirect", NULL, NULL, body, sizeof body, &got, &request);
    size = sizeof text;
    int moved = request && WinHttpQueryOption(request, WINHTTP_OPTION_URL, text, &size) && wide_ends(text, L"/hello");
    if (request) WinHttpCloseHandle(request);
    step(3, "redirect", status == 200 && got == 13 && moved && redirects == 1, redirects);

    /* 4. POST with a body and an added header, both echoed. */
    request = NULL;
    status = fetch(session, L"POST", L"/echo", L"X-Doors: 26H2\r\n", "posted bytes", body, sizeof body, &got, &request);
    size = sizeof text;
    int reflected = request && WinHttpQueryHeaders(request, WINHTTP_QUERY_CUSTOM, L"X-Echo-Doors", text, &size, WINHTTP_NO_HEADER_INDEX) && wide_equal(text, L"26H2");
    DWORD missing = 0;
    size = sizeof text;
    if (request && !WinHttpQueryHeaders(request, WINHTTP_QUERY_CUSTOM, L"X-Absent", text, &size, WINHTTP_NO_HEADER_INDEX)) missing = GetLastError();
    if (request) WinHttpCloseHandle(request);
    step(4, "post", status == 200 && got == 12 && same(body, "posted bytes", 12) && reflected && missing == ERROR_WINHTTP_HEADER_NOT_FOUND, (long)missing);

    /* 5. The session's cookie jar. */
    fetch(session, L"GET", L"/set-cookie", NULL, NULL, body, sizeof body, &got, NULL);
    status = fetch(session, L"GET", L"/cookie", NULL, NULL, body, sizeof body, &got, NULL);
    step(5, "cookies", status == 200 && got == 11 && same(body, "session=abc", 11), got);

    /* 6. 404 is a response, not an error. */
    status = fetch(session, L"GET", L"/nowhere", NULL, NULL, body, sizeof body, &got, NULL);
    step(6, "not-found", status == 404 && got == 7, (long)status);

    /* 7. A Basic challenge, answered with credentials. */
    HINTERNET connect = WinHttpConnect(session, server, port, 0);
    request = WinHttpOpenRequest(connect, L"GET", L"/private", NULL, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, 0);
    DWORD first_status = 0, supported = 0, first = 0, target = 0;
    size = sizeof first_status;
    int challenged = WinHttpSendRequest(request, NULL, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL) &&
        WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, NULL, &first_status, &size, NULL) &&
        first_status == 401 && WinHttpQueryAuthSchemes(request, &supported, &first, &target) && supported == WINHTTP_AUTH_SCHEME_BASIC &&
        target == WINHTTP_AUTH_TARGET_SERVER;
    status = 0;
    size = sizeof status;
    int admitted = challenged && WinHttpSetCredentials(request, WINHTTP_AUTH_TARGET_SERVER, WINHTTP_AUTH_SCHEME_BASIC, L"doors", L"secret", NULL) &&
        WinHttpSendRequest(request, NULL, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL) &&
        WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, NULL, &status, &size, NULL) && status == 200;
    WinHttpCloseHandle(request);
    WinHttpCloseHandle(connect);
    step(7, "basic-auth", challenged && admitted, (long)first_status);

    /* 8. Asynchronous: callbacks drive send, headers and reads. */
    done_event = CreateEventW(NULL, TRUE, FALSE, NULL);
    HINTERNET async_session = WinHttpOpen(L"Doors-HttpProbe/1.0", WINHTTP_ACCESS_TYPE_NO_PROXY, NULL, NULL, WINHTTP_FLAG_ASYNC);
    HINTERNET async_connect = WinHttpConnect(async_session, server, port, 0);
    HINTERNET async_request = WinHttpOpenRequest(async_connect, L"GET", L"/hello", NULL, NULL, NULL, 0);
    WinHttpSetStatusCallback(async_request, async_callback, WINHTTP_CALLBACK_FLAG_ALL_COMPLETIONS, 0);
    int started = WinHttpSendRequest(async_request, NULL, 0, NULL, 0, 0, 0);
    DWORD waited = started ? WaitForSingleObject(done_event, 15000) : WAIT_FAILED;
    WinHttpCloseHandle(async_request);
    WinHttpCloseHandle(async_connect);
    WinHttpCloseHandle(async_session);
    step(8, "async", waited == WAIT_OBJECT_0 && async_error == 0 && async_received == 13 && same(async_body, "hello winhttp", 13), async_received);

    /* 9. Crack and create URLs. */
    URL_COMPONENTS parts;
    memset(&parts, 0, sizeof parts);
    parts.dwStructSize = sizeof parts;
    parts.dwHostNameLength = parts.dwUrlPathLength = parts.dwExtraInfoLength = (DWORD)-1;
    wchar_t created[128];
    DWORD created_length = 128;
    int urls = WinHttpCrackUrl(L"https://Example.COM:8443/a/b?c=d#frag", 0, 0, &parts) && parts.nScheme == INTERNET_SCHEME_HTTPS &&
        parts.nPort == 8443 && parts.dwHostNameLength == 11 && parts.dwUrlPathLength == 4 && parts.dwExtraInfoLength == 9 &&
        WinHttpCreateUrl(&parts, 0, created, &created_length) && wide_equal(created, L"https://Example.COM:8443/a/b?c=d#frag");
    step(9, "urls", urls, (long)created_length);

    /* 10. TLS against a plain-HTTP port fails as a secure failure. */
    connect = WinHttpConnect(session, server, port, 0);
    request = WinHttpOpenRequest(connect, L"GET", L"/hello", NULL, NULL, NULL, WINHTTP_FLAG_SECURE);
    int secured = WinHttpSendRequest(request, NULL, 0, NULL, 0, 0, 0) && WinHttpReceiveResponse(request, NULL);
    DWORD secure_error = secured ? 0 : GetLastError();
    WinHttpCloseHandle(request);
    WinHttpCloseHandle(connect);
    step(10, "secure-failure", !secured && secure_error == ERROR_WINHTTP_SECURE_FAILURE, (long)secure_error);

    /* 11. WINHTTP_OPTION_DECOMPRESSION: WinHTTP asks for gzip and hands
     *     back the decoded body. */
    DWORD decompress = WINHTTP_DECOMPRESSION_FLAG_ALL;
    BOOL decompressing = WinHttpSetOption(session, WINHTTP_OPTION_DECOMPRESSION, &decompress, sizeof decompress);
    status = fetch(session, L"GET", L"/gzip", NULL, NULL, body, sizeof body, &got, NULL);
    step(11, "decompression", decompressing && status == 200 && got == 26 && same(body, "decompressed by the client", 26), got);

    /* 12-14. Digest, NTLM and Negotiate: the 401 names the scheme, the
     *        credentials are set for it and the request is admitted (NTLM
     *        takes its two legs on one connection inside the send). */
    status = authenticated(session, L"/digest", WINHTTP_AUTH_SCHEME_DIGEST, L"doors", body, sizeof body, &got);
    step(12, "digest-auth", status == 200 && got == 9 && same(body, "digest ok", 9), (long)status);
    status = authenticated(session, L"/ntlm", WINHTTP_AUTH_SCHEME_NTLM, L"DOORS\\doors", body, sizeof body, &got);
    step(13, "ntlm-auth", status == 200 && got == 13 && same(body, "ntlm ok doors", 13), (long)status);
    status = authenticated(session, L"/negotiate", WINHTTP_AUTH_SCHEME_NEGOTIATE, L"DOORS\\doors", body, sizeof body, &got);
    step(14, "negotiate-auth", status == 200 && got == 18 && same(body, "negotiate ok doors", 18), (long)status);

    /* 15. Proxy auto-config: the server's PAC script (CONFIG_URL) picks a
     *     proxy by URL and DIRECT for a plain host name; a script that is
     *     not JavaScript and one that cannot be downloaded fail as Windows
     *     says. */
    {
        static wchar_t pac[96], bad[96], gone[96];
        const wchar_t *scheme = L"http://";
        int at = 0;
        for (int i = 0; scheme[i]; i++) pac[at++] = scheme[i];
        for (int i = 0; server[i]; i++) pac[at++] = server[i];
        pac[at++] = L':';
        {
            wchar_t digits[8];
            int n = 0;
            for (unsigned v = port; v; v /= 10) digits[n++] = (wchar_t)(L'0' + v % 10);
            while (n) pac[at++] = digits[--n];
        }
        for (int i = 0; i < at; i++) bad[i] = gone[i] = pac[i];
        for (int t = 0; t < 3; t++) {
            const wchar_t *tail = t == 0 ? L"/proxy.pac" : t == 1 ? L"/bad.pac" : L"/missing.pac";
            wchar_t *slot = t == 0 ? pac : t == 1 ? bad : gone;
            int k = at;
            for (int i = 0; tail[i]; i++) slot[k++] = tail[i];
            slot[k] = 0;
        }
        WINHTTP_AUTOPROXY_OPTIONS options;
        memset(&options, 0, sizeof options);
        options.dwFlags = WINHTTP_AUTOPROXY_CONFIG_URL;
        options.lpszAutoConfigUrl = pac;
        WINHTTP_PROXY_INFO info;
        memset(&info, 0, sizeof info);
        int proxied = WinHttpGetProxyForUrl(session, L"http://www.example.com/via-proxy/page", &options, &info);
        DWORD proxied_error = proxied ? 0 : GetLastError();
        int proxy_ok = proxied && info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY && info.lpszProxy;
        note("pac_access", proxied ? (long)info.dwAccessType : -(long)proxied_error);
        if (proxy_ok) {
            const wchar_t *want = L"proxy.doors.test:3128";
            for (int i = 0; proxy_ok && (want[i] || info.lpszProxy[i]); i++) proxy_ok = want[i] == info.lpszProxy[i];
            note("pac_proxy_matches", proxy_ok);
        }
        note("pac_bypass_null", info.lpszProxyBypass == NULL);
        if (info.lpszProxy) GlobalFree(info.lpszProxy);
        if (info.lpszProxyBypass) GlobalFree(info.lpszProxyBypass);
        memset(&info, 0, sizeof info);
        int direct = WinHttpGetProxyForUrl(session, L"http://intranet/", &options, &info);
        note("pac_direct_access", direct ? (long)info.dwAccessType : -(long)GetLastError());
        int direct_ok = direct && info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY && !info.lpszProxy;
        if (info.lpszProxy) GlobalFree(info.lpszProxy);
        if (info.lpszProxyBypass) GlobalFree(info.lpszProxyBypass);
        options.lpszAutoConfigUrl = bad;
        int bad_ok = !WinHttpGetProxyForUrl(session, L"http://www.example.com/", &options, &info);
        DWORD bad_error = GetLastError();
        options.lpszAutoConfigUrl = gone;
        int gone_ok = !WinHttpGetProxyForUrl(session, L"http://www.example.com/", &options, &info);
        DWORD gone_error = GetLastError();
        note("pac_bad_error", (long)bad_error);
        note("pac_missing_error", (long)gone_error);
        step(15, "proxy-auto-config",
             proxy_ok && direct_ok && bad_ok && bad_error == ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT && gone_ok &&
                 gone_error == ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT,
             (long)proxied_error);
    }

    /* 16-19. HTTPS (TLS 1.3, self-signed certificates): trusted only with
     *        SECURITY_FLAG_IGNORE_UNKNOWN_CA; another host's certificate
     *        and an expired one each also need their own flag. Every
     *        refusal is ERROR_WINHTTP_SECURE_FAILURE with its reasons. */
    {
        DWORD error = 0, after = 0, error2 = 0, after2 = 0;
        const DWORD unknown = SECURITY_FLAG_IGNORE_UNKNOWN_CA;
        status = secure_get(tls_good, unknown, body, sizeof body, &got, &error, &after);
        note("https_security_flags", (long)after);
        step(16, "https", status == 200 && got == 11 && same(body, "hello https", 11) && (after & SECURITY_FLAG_SECURE), (long)error);

        status = secure_get(tls_good, 0, body, sizeof body, &got, &error, &after);
        note("untrusted_reasons", secure_reasons);
        step(17, "untrusted", status == 0 && error == ERROR_WINHTTP_SECURE_FAILURE, (long)error);

        status = secure_get(tls_wrong_host, unknown, body, sizeof body, &got, &error, &after);
        note("wrong_host_reasons", secure_reasons);
        DWORD accepted = secure_get(tls_wrong_host, unknown | SECURITY_FLAG_IGNORE_CERT_CN_INVALID, body, sizeof body, &got, &error2, &after2);
        step(18, "wrong-host",
             status == 0 && error == ERROR_WINHTTP_SECURE_FAILURE && accepted == 200 && got == 11 && same(body, "hello https", 11),
             (long)error);

        status = secure_get(tls_expired, unknown, body, sizeof body, &got, &error, &after);
        note("expired_reasons", secure_reasons);
        accepted = secure_get(tls_expired, unknown | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID, body, sizeof body, &got, &error2, &after2);
        step(19, "expired",
             status == 0 && error == ERROR_WINHTTP_SECURE_FAILURE && accepted == 200 && got == 11 && same(body, "hello https", 11),
             (long)error);
    }

    WinHttpCloseHandle(session);
    put(failed_step ? "HTTP-A FAILED\r\n" : "HTTP-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

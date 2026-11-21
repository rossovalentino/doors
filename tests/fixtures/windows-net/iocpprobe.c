/* CONNECT-A overlapped I/O probe (P15): an ordinary Windows console
 * program (Windows SDK, no C runtime) that drives Winsock the way servers
 * and runtimes do, through an I/O completion port and the Microsoft
 * extension functions:
 *
 *   iocpprobe.exe <host> <tcp echo port> <listen port> [listen address]
 *
 * Packets posted to a port, the extension pointers from WSAIoctl,
 * ConnectEx with data, an overlapped WSARecv seen through its event and
 * WSAGetOverlappedResult, TransmitFile with head and tail buffers, a
 * completion routine run by an alertable wait, CancelIoEx, AcceptEx with
 * the first data (the host connects after "IOCP-A listening"), and
 * DisconnectEx. One "IOCP-A <step> ..." line per step; exit 0 when every
 * step passed, else the number of the first failed one. */
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <windows.h>
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
    int negative = v < 0;
    unsigned long u = negative ? (unsigned long)(-v) : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u && i > 1);
    if (negative) b[--i] = '-';
    put(b + i);
}
static int parse(const char *s) {
    int v = 0;
    while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
    return v;
}
static int same(const char *a, const char *b, int n) {
    for (int i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("IOCP-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static int arguments(char *line, char **argv) {
    int argc = 0;
    while (*line && argc < 8) {
        while (*line == ' ') line++;
        if (!*line) break;
        argv[argc++] = line;
        while (*line && *line != ' ') line++;
        if (*line) *line++ = 0;
    }
    return argc;
}
static void address(struct sockaddr_in *sa, const char *host, int port) {
    ZeroMemory(sa, sizeof *sa);
    sa->sin_family = AF_INET;
    sa->sin_port = htons((u_short)port);
    inet_pton(AF_INET, host, &sa->sin_addr);
}
static void *extension(SOCKET s, GUID id) {
    void *function = NULL;
    DWORD bytes = 0;
    if (WSAIoctl(s, SIO_GET_EXTENSION_FUNCTION_POINTER, &id, sizeof id, &function, sizeof function, &bytes, NULL,
                 NULL) != 0 ||
        bytes != sizeof function)
        return NULL;
    return function;
}
/* The next packet of the port: 1 with it, 0 on failure (error in *error). */
static int packet(HANDLE port, DWORD *bytes, ULONG_PTR *key, OVERLAPPED **ov, DWORD *error) {
    *error = 0;
    if (GetQueuedCompletionStatus(port, bytes, key, ov, 5000)) return 1;
    *error = GetLastError();
    return 0;
}

static DWORD routine_error = 99, routine_bytes;
static int routine_calls;
static void CALLBACK routine(DWORD error, DWORD bytes, LPWSAOVERLAPPED ov, DWORD flags) {
    (void)ov;
    (void)flags;
    routine_error = error;
    routine_bytes = bytes;
    routine_calls++;
}
static char head[] = "head-bytes:";
static char tail[] = ":tail-bytes";

void entry(void) {
    char line[512];
    char *argv[8];
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    {
        LPWSTR wide = GetCommandLineW();
        int i = 0;
        while (wide[i] && i < 511) {
            line[i] = (char)wide[i];
            i++;
        }
        line[i] = 0;
    }
    int argc = arguments(line, argv);
    if (argc < 4) {
        put("usage: iocpprobe host tcp-port listen-port [listen-address]\r\n");
        ExitProcess(100);
    }
    const char *host = argv[1];
    int echo_port = parse(argv[2]);
    int listen_port = parse(argv[3]);
    /* The Windows reference run listens on loopback only. */
    const char *listen_address = argc > 4 ? argv[4] : NULL;
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);

    /* 1. A packet posted to a new port comes back whole; an empty port
     *    times out without one. */
    HANDLE port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, NULL, 0, 0);
    OVERLAPPED marker;
    ZeroMemory(&marker, sizeof marker);
    DWORD bytes = 0, error = 0;
    ULONG_PTR key = 0;
    OVERLAPPED *ov = NULL;
    int posted = port && PostQueuedCompletionStatus(port, 1234, 77, &marker) &&
                 GetQueuedCompletionStatus(port, &bytes, &key, &ov, 1000) && bytes == 1234 && key == 77 &&
                 ov == &marker;
    ov = &marker;
    BOOL empty = GetQueuedCompletionStatus(port, &bytes, &key, &ov, 0);
    error = GetLastError();
    step(1, "port-post", posted && !empty && error == WAIT_TIMEOUT && ov == NULL, (long)error);

    /* 2. The extension functions by GUID. */
    SOCKET client = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, NULL, 0, WSA_FLAG_OVERLAPPED);
    GUID accept_id = WSAID_ACCEPTEX, addresses_id = WSAID_GETACCEPTEXSOCKADDRS, connect_id = WSAID_CONNECTEX,
         disconnect_id = WSAID_DISCONNECTEX, transmit_id = WSAID_TRANSMITFILE;
    LPFN_ACCEPTEX accept_ex = (LPFN_ACCEPTEX)extension(client, accept_id);
    LPFN_GETACCEPTEXSOCKADDRS addresses_ex = (LPFN_GETACCEPTEXSOCKADDRS)extension(client, addresses_id);
    LPFN_CONNECTEX connect_ex = (LPFN_CONNECTEX)extension(client, connect_id);
    LPFN_DISCONNECTEX disconnect_ex = (LPFN_DISCONNECTEX)extension(client, disconnect_id);
    LPFN_TRANSMITFILE transmit_ex = (LPFN_TRANSMITFILE)extension(client, transmit_id);
    GUID unknown = {0x12345678, 1, 2, {3, 4, 5, 6, 7, 8, 9, 10}};
    void *none = extension(client, unknown);
    int all = accept_ex && addresses_ex && connect_ex && disconnect_ex && transmit_ex;
    step(2, "extensions", all && none == NULL, all);
    if (!all) {
        put("IOCP-A FAILED\r\n");
        ExitProcess(2);
    }

    /* 3. ConnectEx on a bound socket with data, through the port. */
    struct sockaddr_in any, echo;
    ZeroMemory(&any, sizeof any);
    any.sin_family = AF_INET;
    address(&echo, host, echo_port);
    bind(client, (struct sockaddr *)&any, sizeof any);
    int associated = CreateIoCompletionPort((HANDLE)client, port, 7, 0) == port;
    static char hello[] = "overlapped-hello";
    OVERLAPPED connect_ov;
    ZeroMemory(&connect_ov, sizeof connect_ov);
    DWORD sent = 0;
    BOOL started = connect_ex(client, (struct sockaddr *)&echo, sizeof echo, hello, 16, &sent, &connect_ov);
    int pending = started || WSAGetLastError() == ERROR_IO_PENDING;
    int connected = pending && packet(port, &bytes, &key, &ov, &error) && key == 7 && ov == &connect_ov && bytes == 16 &&
                    setsockopt(client, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, NULL, 0) == 0;
    step(3, "connect-ex", associated && connected, (long)(error ? error : bytes));

    /* 4. WSARecv with an event: the echo, read through the event and
     *    WSAGetOverlappedResult (the port gets its packet too). */
    char got[64];
    ZeroMemory(got, sizeof got);
    WSABUF buffer = {sizeof got, got};
    WSAOVERLAPPED receive_ov;
    ZeroMemory(&receive_ov, sizeof receive_ov);
    receive_ov.hEvent = WSACreateEvent();
    DWORD flags = 0, received = 0;
    int r = WSARecv(client, &buffer, 1, &received, &flags, &receive_ov, NULL);
    pending = r == 0 || WSAGetLastError() == WSA_IO_PENDING;
    DWORD waited = WSAWaitForMultipleEvents(1, &receive_ov.hEvent, FALSE, 5000, FALSE);
    BOOL done = WSAGetOverlappedResult(client, &receive_ov, &received, FALSE, &flags);
    int drained = packet(port, &bytes, &key, &ov, &error) && ov == &receive_ov;
    /* The echo may arrive in pieces; collect the rest synchronously. */
    while (done && received < 16) {
        int more = recv(client, got + received, (int)(16 - received), 0);
        if (more <= 0) break;
        received += (DWORD)more;
    }
    /* On failure the value says which part failed: 1000 plus 1 not
     * pending, 2 event not signalled, 4 no overlapped result, 8 no port
     * packet for it, 16 short, 32 wrong bytes. */
    long why = (pending ? 0 : 1) | (waited == WSA_WAIT_EVENT_0 ? 0 : 2) | (done ? 0 : 4) | (drained ? 0 : 8) |
               (received == 16 ? 0 : 16) | (same(got, hello, 16) ? 0 : 32);
    step(4, "receive-event", why == 0, why == 0 ? (long)received : 1000 + why);
    WSACloseEvent(receive_ov.hEvent);

    /* 5. TransmitFile with only head and tail buffers; the echo returns both. */
    TRANSMIT_FILE_BUFFERS parts = {head, 11, tail, 11};
    OVERLAPPED transmit_ov;
    ZeroMemory(&transmit_ov, sizeof transmit_ov);
    BOOL transmitted = transmit_ex(client, NULL, 0, 0, &transmit_ov, &parts, 0);
    pending = transmitted || WSAGetLastError() == ERROR_IO_PENDING;
    int finished = pending && packet(port, &bytes, &key, &ov, &error) && ov == &transmit_ov && bytes == 22;
    char back[32];
    int total = 0;
    while (finished && total < 22) {
        int n = recv(client, back + total, 22 - total, 0);
        if (n <= 0) break;
        total += n;
    }
    step(5, "transmit-file", finished && total == 22 && same(back, head, 11) && same(back + 11, tail, 11),
         (long)total);

    /* 6. A completion routine runs in an alertable wait on this thread. */
    SOCKET other = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, NULL, 0, WSA_FLAG_OVERLAPPED);
    int plain = connect(other, (struct sockaddr *)&echo, sizeof echo) == 0 && send(other, "routine!", 8, 0) == 8;
    WSAOVERLAPPED routine_ov;
    ZeroMemory(&routine_ov, sizeof routine_ov);
    ZeroMemory(got, sizeof got);
    WSABUF routine_buffer = {8, got};
    flags = 0;
    r = WSARecv(other, &routine_buffer, 1, &received, &flags, &routine_ov, routine);
    pending = r == 0 || WSAGetLastError() == WSA_IO_PENDING;
    DWORD alerted = 0;
    for (int i = 0; i < 50 && routine_calls == 0; i++) alerted = SleepEx(100, TRUE);
    step(6, "completion-routine", plain && pending && routine_calls == 1 && alerted == WAIT_IO_COMPLETION &&
                                      routine_error == 0 && routine_bytes >= 1,
         (long)routine_bytes);

    /* 7. CancelIoEx ends a receive nobody answers with
     *    ERROR_OPERATION_ABORTED. */
    HANDLE cancel_port = CreateIoCompletionPort((HANDLE)other, NULL, 9, 0);
    while (routine_bytes < 8) { /* the rest of "routine!" */
        int n = recv(other, got, (int)(8 - routine_bytes), 0);
        if (n <= 0) break;
        routine_bytes += (DWORD)n;
    }
    WSAOVERLAPPED cancel_ov;
    ZeroMemory(&cancel_ov, sizeof cancel_ov);
    flags = 0;
    r = WSARecv(other, &buffer, 1, &received, &flags, &cancel_ov, NULL);
    pending = r == SOCKET_ERROR && WSAGetLastError() == WSA_IO_PENDING;
    BOOL cancelled = CancelIoEx((HANDLE)other, &cancel_ov);
    BOOL dequeued = GetQueuedCompletionStatus(cancel_port, &bytes, &key, &ov, 5000);
    error = GetLastError();
    step(7, "cancel", pending && cancelled && !dequeued && error == ERROR_OPERATION_ABORTED && ov == &cancel_ov &&
                          key == 9,
         (long)error);

    /* 8. AcceptEx takes the host's connection with its first four bytes;
     *    an overlapped WSASend answers. */
    SOCKET listener = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, NULL, 0, WSA_FLAG_OVERLAPPED);
    struct sockaddr_in local;
    ZeroMemory(&local, sizeof local);
    local.sin_family = AF_INET;
    local.sin_port = htons((u_short)listen_port);
    if (listen_address) inet_pton(AF_INET, listen_address, &local.sin_addr);
    int listening = bind(listener, (struct sockaddr *)&local, sizeof local) == 0 && listen(listener, 4) == 0 &&
                    CreateIoCompletionPort((HANDLE)listener, port, 11, 0) == port;
    SOCKET accepted = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, NULL, 0, WSA_FLAG_OVERLAPPED);
    static char block[4 + 2 * (sizeof(struct sockaddr_in) + 16)];
    OVERLAPPED accept_ov;
    ZeroMemory(&accept_ov, sizeof accept_ov);
    DWORD first = 0;
    BOOL taken = accept_ex(listener, accepted, block, 4, sizeof(struct sockaddr_in) + 16,
                           sizeof(struct sockaddr_in) + 16, &first, &accept_ov);
    pending = taken || WSAGetLastError() == ERROR_IO_PENDING;
    put("IOCP-A listening\r\n");
    /* The host connects once the firewall lets it (a Security Alert may
     * ask first): the connection gets a minute. */
    error = 0;
    int accepted_one = GetQueuedCompletionStatus(port, &bytes, &key, &ov, 60000);
    if (!accepted_one) error = GetLastError();
    int arrived = pending && accepted_one && key == 11 && ov == &accept_ov && bytes == 4 && same(block, "ping", 4);
    struct sockaddr *mine = NULL, *theirs = NULL;
    int mine_length = 0, theirs_length = 0;
    if (arrived)
        addresses_ex(block, 4, sizeof(struct sockaddr_in) + 16, sizeof(struct sockaddr_in) + 16, &mine, &mine_length,
                     &theirs, &theirs_length);
    int addressed = mine && theirs && theirs->sa_family == AF_INET &&
                    ((struct sockaddr_in *)mine)->sin_port == htons((u_short)listen_port) &&
                    ((struct sockaddr_in *)theirs)->sin_port != 0;
    int updated = setsockopt(accepted, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, (char *)&listener, sizeof listener) == 0;
    int answered = 0;
    if (CreateIoCompletionPort((HANDLE)accepted, port, 12, 0) == port) {
        WSABUF pong = {4, "pong"};
        WSAOVERLAPPED send_ov;
        ZeroMemory(&send_ov, sizeof send_ov);
        r = WSASend(accepted, &pong, 1, &sent, 0, &send_ov, NULL);
        answered = (r == 0 || WSAGetLastError() == WSA_IO_PENDING) && packet(port, &bytes, &key, &ov, &error) &&
                   key == 12 && ov == &send_ov && bytes == 4;
    }
    step(8, "accept-ex", listening && arrived && addressed && updated && answered, (long)(error ? error : bytes));

    /* 9. DisconnectEx ends the ConnectEx connection: nothing more can be
     *    sent on it. */
    OVERLAPPED disconnect_ov;
    ZeroMemory(&disconnect_ov, sizeof disconnect_ov);
    BOOL ended = disconnect_ex(client, &disconnect_ov, 0, 0);
    pending = ended || WSAGetLastError() == ERROR_IO_PENDING;
    int disconnected = pending && packet(port, &bytes, &key, &ov, &error) && ov == &disconnect_ov && key == 7;
    int refused = send(client, "x", 1, 0) == SOCKET_ERROR;
    step(9, "disconnect-ex", disconnected && refused, (long)(error ? error : WSAGetLastError()));

    Sleep(500);
    closesocket(accepted);
    closesocket(listener);
    closesocket(other);
    closesocket(client);
    CloseHandle(cancel_port);
    CloseHandle(port);
    WSACleanup();
    put(failed_step ? "IOCP-A FAILED\r\n" : "IOCP-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

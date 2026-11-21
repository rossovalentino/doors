/* CONNECT-A Winsock probe (P15): an ordinary Windows console program built
 * with the Windows SDK and no C runtime. It exercises ws2_32 against the
 * host endpoints the acceptance run passes on its command line:
 *
 *   netprobe.exe <host> <tcp echo port> <udp echo port> <listen port> <reset port>
 *
 * and prints one "CONNECT-A <step> ..." line per step. The exit code is 0
 * when every step passed, else the number of the first failed step. */
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <intrin.h>

/* No C runtime: the compiler may still emit memset for zeroing. */
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
    put("CONNECT-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
/* Split the command line into at most 8 arguments (no quoting needed). */
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

void entry(void) {
    char line[512];
    char *argv[8];
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    {
        /* GetCommandLineA is not in Door's kernel32 yet: copy the wide one. */
        LPWSTR wide = GetCommandLineW();
        int i = 0;
        while (wide[i] && i < 511) {
            line[i] = (char)wide[i];
            i++;
        }
        line[i] = 0;
    }
    int argc = arguments(line, argv);
    if (argc < 6) {
        put("usage: netprobe host tcp udp listen closed\r\n");
        ExitProcess(100);
    }
    const char *host = argv[1];
    int tcp_port = parse(argv[2]), udp_port = parse(argv[3]);
    int listen_port = parse(argv[4]), reset_port = parse(argv[5]);

    /* 1. Startup. */
    WSADATA data;
    int r = WSAStartup(MAKEWORD(2, 2), &data);
    step(1, "startup", r == 0 && data.wVersion == MAKEWORD(2, 2), data.wVersion);

    /* 2. Host name and a numeric lookup with a service. */
    char name[64];
    r = gethostname(name, sizeof name);
    step(2, "gethostname", r == 0 && name[0] != 0, length(name));
    struct addrinfo hints, *found = NULL;
    ZeroMemory(&hints, sizeof hints);
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    r = getaddrinfo(host, argv[2], &hints, &found);
    int numeric_ok = r == 0 && found && found->ai_family == AF_INET &&
        ntohs(((struct sockaddr_in *)found->ai_addr)->sin_port) == tcp_port;
    if (found) freeaddrinfo(found);
    hints.ai_flags = AI_NUMERICHOST;
    found = NULL;
    int refused_name = getaddrinfo("not-a-number", NULL, &hints, &found);
    step(3, "getaddrinfo", numeric_ok && refused_name == WSAHOST_NOT_FOUND, refused_name);
    hints.ai_flags = 0;
    found = NULL;
    r = getaddrinfo("localhost", "http", &hints, &found);
    step(4, "localhost", r == 0 && found &&
         ((struct sockaddr_in *)found->ai_addr)->sin_addr.s_addr == htonl(INADDR_LOOPBACK) &&
         ntohs(((struct sockaddr_in *)found->ai_addr)->sin_port) == 80, r);
    if (found) freeaddrinfo(found);

    /* 5. Blocking TCP: echo, names, half-close, end of stream. */
    SOCKET s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in to, me, peer;
    address(&to, host, tcp_port);
    r = connect(s, (struct sockaddr *)&to, sizeof to);
    const char hello[] = "hello winsock";
    int sent = r == 0 ? send(s, hello, sizeof hello - 1, 0) : -1;
    char back[64];
    int got = 0;
    while (got < (int)sizeof hello - 1) {
        int n = recv(s, back + got, (int)sizeof back - got, 0);
        if (n <= 0) break;
        got += n;
    }
    int me_len = sizeof me, peer_len = sizeof peer;
    int named = getsockname(s, (struct sockaddr *)&me, &me_len) == 0 && me.sin_port != 0 &&
        getpeername(s, (struct sockaddr *)&peer, &peer_len) == 0 && ntohs(peer.sin_port) == tcp_port;
    shutdown(s, SD_SEND);
    int end = recv(s, back, sizeof back, 0);
    closesocket(s);
    step(5, "tcp-echo", r == 0 && sent == (int)sizeof hello - 1 && got == sent &&
         same(back, hello, got) && named && end == 0, got);

    /* 6. Nonblocking connect completed through select(). */
    s = socket(AF_INET, SOCK_STREAM, 0);
    u_long on = 1;
    ioctlsocket(s, FIONBIO, &on);
    r = connect(s, (struct sockaddr *)&to, sizeof to);
    int pending = r == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK;
    fd_set writable;
    FD_ZERO(&writable);
    FD_SET(s, &writable);
    struct timeval wait = {10, 0};
    int ready = select(0, NULL, &writable, NULL, &wait);
    int error = -1, error_len = sizeof error;
    getsockopt(s, SOL_SOCKET, SO_ERROR, (char *)&error, &error_len);
    int marked = FD_ISSET(s, &writable);
    closesocket(s);
    step(6, "nonblocking", pending && ready == 1 && marked && error == 0, ready);

    /* 7. The server closes with our byte unread, which resets the
     * connection: recv fails with WSAECONNRESET, not an orderly 0. */
    s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in resetting;
    address(&resetting, host, reset_port);
    r = connect(s, (struct sockaddr *)&resetting, sizeof resetting);
    int poked = r == 0 && send(s, "x", 1, 0) == 1;
    int reset_result = 0, reset_error = 0;
    for (int i = 0; poked && i < 50; i++) {
        reset_result = recv(s, back, sizeof back, 0);
        reset_error = WSAGetLastError();
        if (reset_result <= 0) break;
    }
    closesocket(s);
    step(7, "reset", poked && reset_result == SOCKET_ERROR && reset_error == WSAECONNRESET, reset_error);

    /* 8. UDP echo with a receive timeout. */
    s = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    DWORD timeout = 5000;
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof timeout);
    struct sockaddr_in udp, from;
    address(&udp, host, udp_port);
    const char datagram[] = "datagram";
    sent = sendto(s, datagram, sizeof datagram - 1, 0, (struct sockaddr *)&udp, sizeof udp);
    int from_len = sizeof from;
    got = recvfrom(s, back, sizeof back, 0, (struct sockaddr *)&from, &from_len);
    int small_ok = sent == (int)sizeof datagram - 1 && got == sent && same(back, datagram, got) &&
        ntohs(from.sin_port) == udp_port;
    /* A datagram larger than one IPC message, then one cut to fit. */
    static char large[1400], echoed[1400];
    for (int i = 0; i < (int)sizeof large; i++) large[i] = (char)(i * 7 + 3);
    int large_sent = sendto(s, large, sizeof large, 0, (struct sockaddr *)&udp, sizeof udp);
    int large_got = recvfrom(s, echoed, sizeof echoed, 0, NULL, NULL);
    int large_ok = large_sent == (int)sizeof large && large_got == large_sent && same(large, echoed, large_got);
    sendto(s, large, 300, 0, (struct sockaddr *)&udp, sizeof udp);
    from_len = sizeof from;
    int cut = recvfrom(s, echoed, 100, 0, (struct sockaddr *)&from, &from_len);
    int cut_ok = cut == SOCKET_ERROR && WSAGetLastError() == WSAEMSGSIZE && same(large, echoed, 100) &&
        ntohs(from.sin_port) == udp_port;
    closesocket(s);
    step(8, "udp-echo", small_ok && large_ok && cut_ok, large_got);

    /* 9. A server the host connects to through the forwarded port. */
    SOCKET listener = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in any;
    ZeroMemory(&any, sizeof any);
    any.sin_family = AF_INET;
    any.sin_port = htons((u_short)listen_port);
    r = bind(listener, (struct sockaddr *)&any, sizeof any);
    r = r == 0 ? listen(listener, SOMAXCONN) : r;
    put("CONNECT-A listening ");
    number(listen_port);
    put("\r\n");
    fd_set readable;
    FD_ZERO(&readable);
    FD_SET(listener, &readable);
    struct timeval server_wait = {60, 0};
    ready = r == 0 ? select(0, &readable, NULL, NULL, &server_wait) : -1;
    SOCKET client = ready == 1 ? accept(listener, NULL, NULL) : INVALID_SOCKET;
    got = 0;
    while (client != INVALID_SOCKET && got < 4) {
        int n = recv(client, back + got, (int)sizeof back - got, 0);
        if (n <= 0) break;
        got += n;
    }
    int answered = got == 4 && same(back, "ping", 4) && send(client, "pong", 4, 0) == 4;
    /* 10. WSAPoll sees the peer's close after the answer. */
    WSAPOLLFD poll = {client, POLLRDNORM, 0};
    int polled = client != INVALID_SOCKET ? WSAPoll(&poll, 1, 10000) : -1;
    if (client != INVALID_SOCKET) closesocket(client);
    closesocket(listener);
    step(9, "server", r == 0 && answered, got);
    step(10, "wsapoll", polled == 1 && (poll.revents & (POLLRDNORM | POLLHUP)), poll.revents);

    /* 11. Cleanup, then a call without startup fails as Windows does. */
    r = WSACleanup();
    SOCKET late = socket(AF_INET, SOCK_STREAM, 0);
    step(11, "cleanup", r == 0 && late == INVALID_SOCKET && WSAGetLastError() == WSANOTINITIALISED, WSAGetLastError());

    put(failed_step ? "CONNECT-A FAILED\r\n" : "CONNECT-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

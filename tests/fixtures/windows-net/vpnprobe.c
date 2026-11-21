/* VPN-A probe (P15): an ordinary Windows console program (Windows SDK, no
 * C runtime) that checks traffic while a WireGuard connection is up and
 * after it is down. The acceptance run's host peer serves, inside the
 * tunnel, TCP and UDP echo on 10.8.0.1 port 7 and DNS for vpn.doors.test;
 * the host itself serves TCP echo on 10.0.2.2 (QEMU's host address):
 *
 *   vpnprobe.exe tunnel <host tcp echo port>
 *   vpnprobe.exe direct <host tcp echo port>
 *
 * Prints one "VPN-A <step> ..." line per step; exits 0 when all passed,
 * else the number of the first failed step. */
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
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
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("VPN-A ");
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
/* A TCP echo of `count` patterned bytes; answers the bytes echoed intact
 * and stores the local address. */
static int echo(const char *host, int port, int count, struct sockaddr_in *local) {
    static char sent[4096], back[4096];
    SOCKET s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    DWORD timeout = 10000;
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof timeout);
    struct sockaddr_in to;
    address(&to, host, port);
    if (connect(s, (struct sockaddr *)&to, sizeof to) != 0) {
        closesocket(s);
        return -1;
    }
    int local_len = sizeof *local;
    getsockname(s, (struct sockaddr *)local, &local_len);
    for (int i = 0; i < count; i++) sent[i] = (char)('a' + i % 26);
    int put_bytes = 0;
    while (put_bytes < count) {
        int n = send(s, sent + put_bytes, count - put_bytes, 0);
        if (n <= 0) break;
        put_bytes += n;
    }
    int got = 0;
    while (got < count) {
        int n = recv(s, back + got, count - got, 0);
        if (n <= 0) break;
        got += n;
    }
    closesocket(s);
    for (int i = 0; i < got; i++)
        if (back[i] != sent[i]) return i;
    return got;
}
/* A nonblocking connect that must not complete within three seconds. */
static int unreachable(const char *host, int port) {
    SOCKET s = socket(AF_INET, SOCK_STREAM, 0);
    u_long on = 1;
    ioctlsocket(s, FIONBIO, &on);
    struct sockaddr_in to;
    address(&to, host, port);
    connect(s, (struct sockaddr *)&to, sizeof to);
    fd_set writable;
    FD_ZERO(&writable);
    FD_SET(s, &writable);
    struct timeval wait = {3, 0};
    int ready = select(0, NULL, &writable, NULL, &wait);
    int error = -1, error_len = sizeof error;
    getsockopt(s, SOL_SOCKET, SO_ERROR, (char *)&error, &error_len);
    closesocket(s);
    /* Not connected: timed out, or refused/reset. */
    return ready == 0 || error != 0;
}
static unsigned long bytes(const struct sockaddr_in *sa) {
    return ntohl(sa->sin_addr.s_addr);
}

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
    if (argc < 3 || (argv[1][0] != 't' && argv[1][0] != 'd')) {
        put("usage: vpnprobe tunnel|direct <host tcp echo port>\r\n");
        ExitProcess(100);
    }
    int tunnel = argv[1][0] == 't';
    int host_port = parse(argv[2]);
    WSADATA data;
    int r = WSAStartup(MAKEWORD(2, 2), &data);
    step(1, "startup", r == 0, r);
    struct sockaddr_in local;
    if (tunnel) {
        /* 2. The tunnel's DNS server answers the name. */
        struct addrinfo hints, *found = NULL;
        ZeroMemory(&hints, sizeof hints);
        hints.ai_family = AF_INET;
        hints.ai_socktype = SOCK_STREAM;
        r = getaddrinfo("vpn.doors.test", "7", &hints, &found);
        unsigned long resolved = r == 0 && found ? bytes((struct sockaddr_in *)found->ai_addr) : 0;
        if (found) freeaddrinfo(found);
        step(2, "dns", resolved == 0x0A080001, r);
        /* 3. TCP through the tunnel, several segments, from 10.8.0.2. */
        ZeroMemory(&local, sizeof local);
        int got = echo("10.8.0.1", 7, 4000, &local);
        step(3, "tcp-echo", got == 4000 && bytes(&local) == 0x0A080002, got);
        /* 4. UDP through the tunnel. */
        SOCKET s = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
        DWORD timeout = 5000;
        setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof timeout);
        struct sockaddr_in to, from;
        address(&to, "10.8.0.1", 7);
        const char datagram[] = "datagram through wireguard";
        int sent = sendto(s, datagram, sizeof datagram - 1, 0, (struct sockaddr *)&to, sizeof to);
        char back[64];
        int from_len = sizeof from;
        int n = recvfrom(s, back, sizeof back, 0, (struct sockaddr *)&from, &from_len);
        closesocket(s);
        step(4, "udp-echo", sent == (int)sizeof datagram - 1 && n == sent && bytes(&from) == 0x0A080001, n);
        /* 5. IPv6 through the tunnel: the server's fd00:8::1 answers from
         * inside it, the datagram leaving from the tunnel's fd00:8::2. */
        SOCKET s6 = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
        setsockopt(s6, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof timeout);
        struct sockaddr_in6 to6, from6;
        ZeroMemory(&to6, sizeof to6);
        to6.sin6_family = AF_INET6;
        to6.sin6_port = htons(7);
        inet_pton(AF_INET6, "fd00:8::1", &to6.sin6_addr);
        const char datagram6[] = "ipv6 through wireguard";
        int sent6 = sendto(s6, datagram6, sizeof datagram6 - 1, 0, (struct sockaddr *)&to6, sizeof to6);
        int from6_len = sizeof from6;
        int n6 = recvfrom(s6, back, sizeof back, 0, (struct sockaddr *)&from6, &from6_len);
        closesocket(s6);
        IN6_ADDR server6;
        inet_pton(AF_INET6, "fd00:8::1", &server6);
        int same_server = 1;
        for (int i = 0; i < 16; i++)
            if (from6.sin6_addr.u.Byte[i] != server6.u.Byte[i]) same_server = 0;
        step(5, "udp6-echo", n6 == sent6 && sent6 == (int)sizeof datagram6 - 1 && same_server, n6);
        /* 6. With 0.0.0.0/0 even the host's own address is routed into the
         * tunnel (as WireGuard routes it), where nothing serves that port:
         * the host's echo server must not be reached directly. */
        step(6, "all-tunneled", unreachable("10.0.2.2", host_port), host_port);
    } else {
        /* 2. The host directly, from the adapter's own address. */
        ZeroMemory(&local, sizeof local);
        int got = echo("10.0.2.2", host_port, 1000, &local);
        step(2, "direct", got == 1000 && (bytes(&local) >> 24) == 10 && bytes(&local) != 0x0A080002, got);
        /* 3. The tunnel's side is gone: 10.8.0.1 is routed out of the
         * Ethernet adapter again (a connected UDP socket's local address),
         * not from the tunnel's 10.8.0.2. (Whether a TCP connect there is
         * refused depends on the host's own network.) */
        SOCKET s = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
        struct sockaddr_in to;
        address(&to, "10.8.0.1", 7);
        ZeroMemory(&local, sizeof local);
        int local_len = sizeof local;
        int routed = connect(s, (struct sockaddr *)&to, sizeof to) == 0 &&
                     getsockname(s, (struct sockaddr *)&local, &local_len) == 0;
        closesocket(s);
        step(3, "tunnel-gone", routed && (bytes(&local) >> 24) == 10 && bytes(&local) != 0x0A080002, (long)(bytes(&local) & 0xFF));
    }
    WSACleanup();
    put(failed_step ? "VPN-A FAILED\r\n" : "VPN-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

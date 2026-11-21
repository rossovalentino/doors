/* CONNECT-A IPv6 probe (P15): an ordinary Windows console program (Windows
 * SDK, no C runtime) that uses Winsock's IPv6 side:
 *
 *   v6probe.exe <v6 host> <tcp6 echo port> <udp6 echo port> <v4 host> <tcp4 echo port>
 *
 * Address text both ways, WSAStringToAddress/WSAAddressToString with a
 * port, getaddrinfo for localhost and the passive defaults, an adapter
 * with an IPv6 address, TCP and UDP echo over IPv6, and a dual-stack socket
 * (IPV6_V6ONLY off) reaching an IPv4 server through its IPv4-mapped
 * address. One "V6-A <step> ..." line per step; exit 0 when every step
 * passed, else the number of the first failed one. */
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <iphlpapi.h>
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
static int same(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static int same_bytes(const void *a, const void *b, int n) {
    for (int i = 0; i < n; i++)
        if (((const unsigned char *)a)[i] != ((const unsigned char *)b)[i]) return 0;
    return 1;
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("V6-A ");
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
/* Echo `n` bytes through a connected socket; 1 when they come back. */
static int echoed(SOCKET s, const char *data, int n) {
    char back[64];
    if (send(s, data, n, 0) != n) return 0;
    int got = 0;
    while (got < n) {
        int r = recv(s, back + got, n - got, 0);
        if (r <= 0) return 0;
        got += r;
    }
    return same_bytes(back, data, n);
}
static unsigned char adapters[32 * 1024];

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
    if (arguments(line, argv) < 6) {
        put("usage: v6probe v6-host tcp6-port udp6-port v4-host tcp4-port\r\n");
        ExitProcess(100);
    }
    const char *host6 = argv[1];
    int tcp6 = parse(argv[2]), udp6 = parse(argv[3]);
    const char *host4 = argv[4];
    int tcp4 = parse(argv[5]);
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);

    /* 1. IPv6 text both ways, in Windows' canonical form. */
    static const char *const forms[][2] = {
        {"FEC0:0:0:0:0:0:0:2", "fec0::2"},
        {"fe80::5054:ff:fed0:501", "fe80::5054:ff:fed0:501"},
        {"2001:db8:0:0:1:0:0:1", "2001:db8::1:0:0:1"},
        {"::ffff:10.0.2.2", "::ffff:10.0.2.2"},
        {"::", "::"},
    };
    int texts = 0;
    for (int i = 0; i < 5; i++) {
        IN6_ADDR a;
        char back[64];
        if (inet_pton(AF_INET6, forms[i][0], &a) == 1 && inet_ntop(AF_INET6, &a, back, sizeof back) &&
            same(back, forms[i][1]))
            texts++;
    }
    IN6_ADDR bad;
    int refused = inet_pton(AF_INET6, "1::2::3", &bad) == 0 && inet_pton(AF_INET6, "12345::", &bad) == 0;
    step(1, "text", texts == 5 && refused, texts);

    /* 2. "[addr]:port" into a SOCKADDR_IN6 and back. */
    SOCKADDR_IN6 sa;
    INT sa_length = sizeof sa;
    wchar_t text[64];
    DWORD text_length = 64;
    int converted = WSAStringToAddressW(L"[fec0::2]:8080", AF_INET6, NULL, (LPSOCKADDR)&sa, &sa_length) == 0 &&
                    sa.sin6_family == AF_INET6 && ntohs(sa.sin6_port) == 8080 && sa_length == sizeof sa &&
                    WSAAddressToStringW((LPSOCKADDR)&sa, sizeof sa, NULL, text, &text_length) == 0;
    int round = 0;
    if (converted) {
        static const wchar_t want[] = L"[fec0::2]:8080";
        round = 1;
        for (int i = 0; want[i] || text[i]; i++)
            if (text[i] != want[i]) round = 0;
    }
    step(2, "string-address", converted && round, (long)text_length);

    /* 3. getaddrinfo: localhost and the passive defaults of both families. */
    ADDRINFOA hints, *list = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET6;
    int loopback = getaddrinfo("localhost", "80", &hints, &list) == 0 && list && list->ai_family == AF_INET6 &&
                   list->ai_addrlen == sizeof(SOCKADDR_IN6) &&
                   IN6_IS_ADDR_LOOPBACK(&((SOCKADDR_IN6 *)list->ai_addr)->sin6_addr) &&
                   ntohs(((SOCKADDR_IN6 *)list->ai_addr)->sin6_port) == 80;
    if (list) freeaddrinfo(list);
    list = NULL;
    hints.ai_family = AF_UNSPEC;
    hints.ai_flags = AI_PASSIVE;
    int any6 = 0, any4 = 0;
    if (getaddrinfo(NULL, "7000", &hints, &list) == 0) {
        for (ADDRINFOA *p = list; p; p = p->ai_next) {
            if (p->ai_family == AF_INET6 && IN6_IS_ADDR_UNSPECIFIED(&((SOCKADDR_IN6 *)p->ai_addr)->sin6_addr)) any6 = 1;
            if (p->ai_family == AF_INET && ((SOCKADDR_IN *)p->ai_addr)->sin_addr.s_addr == 0) any4 = 1;
        }
        freeaddrinfo(list);
    }
    step(3, "getaddrinfo", loopback && any6 && any4, loopback | any6 << 1 | any4 << 2);

    /* 4. An adapter carries an IPv6 address (GetAdaptersAddresses). */
    ULONG size = sizeof adapters;
    ULONG r = GetAdaptersAddresses(AF_INET6, GAA_FLAG_SKIP_DNS_SERVER, NULL, (PIP_ADAPTER_ADDRESSES)adapters, &size);
    int v6_addresses = 0;
    for (PIP_ADAPTER_ADDRESSES a = r == NO_ERROR ? (PIP_ADAPTER_ADDRESSES)adapters : NULL; a; a = a->Next)
        for (PIP_ADAPTER_UNICAST_ADDRESS u = a->FirstUnicastAddress; u; u = u->Next)
            if (u->Address.lpSockaddr->sa_family == AF_INET6) v6_addresses++;
    step(4, "adapters", r == NO_ERROR && v6_addresses > 0, (long)r);

    /* 5. TCP echo over IPv6; the peer comes back as a SOCKADDR_IN6. */
    SOCKADDR_IN6 server;
    memset(&server, 0, sizeof server);
    server.sin6_family = AF_INET6;
    server.sin6_port = htons((u_short)tcp6);
    inet_pton(AF_INET6, host6, &server.sin6_addr);
    SOCKET tcp = socket(AF_INET6, SOCK_STREAM, IPPROTO_TCP);
    int connected = tcp != INVALID_SOCKET && connect(tcp, (struct sockaddr *)&server, sizeof server) == 0;
    SOCKADDR_IN6 peer;
    int peer_length = sizeof peer;
    int named = connected && getpeername(tcp, (struct sockaddr *)&peer, &peer_length) == 0 &&
                peer_length == sizeof peer && peer.sin6_family == AF_INET6 &&
                same_bytes(&peer.sin6_addr, &server.sin6_addr, 16) && peer.sin6_port == server.sin6_port;
    int tcp_ok = connected && named && echoed(tcp, "over ipv6", 9);
    closesocket(tcp);
    step(5, "tcp6-echo", tcp_ok, connected | named << 1);

    /* 6. UDP echo over IPv6, the sender reported as the server. */
    SOCKET udp = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
    SOCKADDR_IN6 to = server;
    to.sin6_port = htons((u_short)udp6);
    DWORD timeout = 3000;
    setsockopt(udp, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof timeout);
    int sent = sendto(udp, "datagram6", 9, 0, (struct sockaddr *)&to, sizeof to);
    char back[32];
    SOCKADDR_IN6 from;
    int from_length = sizeof from;
    int got = recvfrom(udp, back, sizeof back, 0, (struct sockaddr *)&from, &from_length);
    int udp_ok = sent == 9 && got == 9 && same_bytes(back, "datagram6", 9) && from.sin6_family == AF_INET6 &&
                 from.sin6_port == to.sin6_port;
    closesocket(udp);
    step(6, "udp6-echo", udp_ok, got);

    /* 7. Dual stack: IPV6_V6ONLY is on by default; turned off, an AF_INET6
     *    socket reaches an IPv4 server at ::ffff:a.b.c.d. */
    SOCKET dual = socket(AF_INET6, SOCK_STREAM, IPPROTO_TCP);
    DWORD only = 7;
    int only_length = sizeof only;
    int default_on = getsockopt(dual, IPPROTO_IPV6, IPV6_V6ONLY, (char *)&only, &only_length) == 0 && only == 1;
    DWORD off = 0;
    int switched = setsockopt(dual, IPPROTO_IPV6, IPV6_V6ONLY, (const char *)&off, sizeof off) == 0;
    SOCKADDR_IN6 mapped;
    memset(&mapped, 0, sizeof mapped);
    mapped.sin6_family = AF_INET6;
    mapped.sin6_port = htons((u_short)tcp4);
    IN_ADDR v4;
    inet_pton(AF_INET, host4, &v4);
    mapped.sin6_addr.s6_addr[10] = 0xff;
    mapped.sin6_addr.s6_addr[11] = 0xff;
    for (int i = 0; i < 4; i++) mapped.sin6_addr.s6_addr[12 + i] = ((unsigned char *)&v4)[i];
    int dual_ok = default_on && switched && connect(dual, (struct sockaddr *)&mapped, sizeof mapped) == 0 &&
                  echoed(dual, "mapped!", 7);
    closesocket(dual);
    step(7, "dual-stack", dual_ok, default_on | switched << 1);

    /* 8. The IPv6 tables list this process's AF_INET6 sockets (and the
     *    IPv4 ones do not); the basic classes do not exist for AF_INET6. */
    SOCKET open6 = socket(AF_INET6, SOCK_STREAM, IPPROTO_TCP);
    int open_ok = open6 != INVALID_SOCKET && connect(open6, (struct sockaddr *)&server, sizeof server) == 0;
    SOCKET bound6 = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
    SOCKADDR_IN6 wildcard;
    memset(&wildcard, 0, sizeof wildcard);
    wildcard.sin6_family = AF_INET6;
    int bound_ok = bound6 != INVALID_SOCKET && bind(bound6, (struct sockaddr *)&wildcard, sizeof wildcard) == 0;
    SOCKADDR_IN6 bound_name;
    int bound_length = sizeof bound_name;
    getsockname(bound6, (struct sockaddr *)&bound_name, &bound_length);
    DWORD me = GetCurrentProcessId();
    static BYTE table[16384];
    size = sizeof table;
    int tcp_listed = 0, udp_listed = 0, v4_clean = 1;
    if (GetExtendedTcpTable(table, &size, FALSE, AF_INET6, TCP_TABLE_OWNER_PID_CONNECTIONS, 0) == NO_ERROR) {
        MIB_TCP6TABLE_OWNER_PID *t = (MIB_TCP6TABLE_OWNER_PID *)table;
        for (DWORD i = 0; i < t->dwNumEntries; i++)
            if (t->table[i].dwOwningPid == me && t->table[i].dwRemotePort == (DWORD)htons((u_short)tcp6) &&
                t->table[i].dwState == MIB_TCP_STATE_ESTAB)
                tcp_listed = 1;
    }
    size = sizeof table;
    if (GetExtendedUdpTable(table, &size, FALSE, AF_INET6, UDP_TABLE_OWNER_PID, 0) == NO_ERROR) {
        MIB_UDP6TABLE_OWNER_PID *t = (MIB_UDP6TABLE_OWNER_PID *)table;
        for (DWORD i = 0; i < t->dwNumEntries; i++)
            if (t->table[i].dwOwningPid == me && t->table[i].dwLocalPort == (DWORD)bound_name.sin6_port)
                udp_listed = 1;
    }
    size = sizeof table;
    if (GetExtendedTcpTable(table, &size, FALSE, AF_INET, TCP_TABLE_OWNER_PID_CONNECTIONS, 0) == NO_ERROR) {
        MIB_TCPTABLE_OWNER_PID *t = (MIB_TCPTABLE_OWNER_PID *)table;
        for (DWORD i = 0; i < t->dwNumEntries; i++)
            if (t->table[i].dwOwningPid == me && t->table[i].dwRemotePort == (DWORD)htons((u_short)tcp6))
                v4_clean = 0;
    }
    size = sizeof table;
    int basic_refused =
        GetExtendedTcpTable(table, &size, FALSE, AF_INET6, TCP_TABLE_BASIC_ALL, 0) == ERROR_INVALID_PARAMETER;
    closesocket(open6);
    closesocket(bound6);
    step(8, "tables", open_ok && bound_ok && tcp_listed && udp_listed && v4_clean && basic_refused,
         tcp_listed | udp_listed << 1 | v4_clean << 2 | basic_refused << 3);

    WSACleanup();
    put(failed_step ? "V6-A FAILED\r\n" : "V6-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

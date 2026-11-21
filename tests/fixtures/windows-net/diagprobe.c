/* CONNECT-A diagnostics probe (P15): an ordinary Windows console program
 * (Windows SDK, no C runtime) that uses iphlpapi's ICMP echo and the TCP
 * and UDP tables, as ping and netstat do:
 *
 *   diagprobe.exe <host> <tcp echo port>
 *
 * ICMP echo to the host with the caller's data, an echo nobody answers,
 * this process's listening TCP socket and bound UDP socket in the owner
 * tables, its connection to the TCP echo server in GetTcpTable2, the
 * protocol statistics and the default gateway in the ARP table.
 * One "DIAG-A <step> ..." line per step; exit 0 when every step passed,
 * else the number of the first failed one. */
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <iphlpapi.h>
#include <icmpapi.h>
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
    put("DIAG-A ");
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
static unsigned char table[64 * 1024];
static unsigned char reply[512];

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
    if (arguments(line, argv) < 3) {
        put("usage: diagprobe host tcp-port\r\n");
        ExitProcess(100);
    }
    const char *host = argv[1];
    int tcp_port = parse(argv[2]);
    IN_ADDR target;
    inet_pton(AF_INET, host, &target);
    DWORD me = GetCurrentProcessId();
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);

    /* 1. An echo of the caller's data comes back from the host. */
    HANDLE icmp = IcmpCreateFile();
    static char data[] = "doors-probe-data";
    DWORD replies = icmp != INVALID_HANDLE_VALUE
        ? IcmpSendEcho(icmp, target.S_un.S_addr, data, 16, NULL, reply, sizeof reply, 4000)
        : 0;
    PICMP_ECHO_REPLY echo = (PICMP_ECHO_REPLY)reply;
    int echoed = replies == 1 && echo->Status == IP_SUCCESS && echo->Address == target.S_un.S_addr &&
                 echo->DataSize == 16;
    for (int i = 0; echoed && i < 16; i++) echoed = ((char *)echo->Data)[i] == data[i];
    step(1, "icmp-echo", echoed, replies == 1 ? (long)echo->Status : (long)GetLastError());

    /* 2. An address nobody answers (TEST-NET-1) times out or is reported
     *    unreachable; no reply is invented. */
    IN_ADDR nobody;
    inet_pton(AF_INET, "192.0.2.1", &nobody);
    DWORD none = IcmpSendEcho(icmp, nobody.S_un.S_addr, data, 16, NULL, reply, sizeof reply, 800);
    DWORD why = none ? echo->Status : GetLastError();
    int unanswered = (none == 0 && why == IP_REQ_TIMED_OUT) ||
                     (none == 1 && echo->Status >= IP_DEST_NET_UNREACHABLE && echo->Status <= IP_DEST_PORT_UNREACHABLE);
    step(2, "icmp-unanswered", unanswered && IcmpCloseHandle(icmp), (long)why);

    /* 3. This process's listening socket in the owner TCP table. */
    SOCKET listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in local;
    ZeroMemory(&local, sizeof local);
    local.sin_family = AF_INET;
    int size = sizeof local;
    int listening = bind(listener, (struct sockaddr *)&local, sizeof local) == 0 && listen(listener, 1) == 0 &&
                    getsockname(listener, (struct sockaddr *)&local, &size) == 0;
    DWORD bytes = sizeof table;
    DWORD r = GetExtendedTcpTable(table, &bytes, TRUE, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
    PMIB_TCPTABLE_OWNER_PID tcp = (PMIB_TCPTABLE_OWNER_PID)table;
    int found = 0;
    for (DWORD i = 0; r == NO_ERROR && i < tcp->dwNumEntries; i++) {
        MIB_TCPROW_OWNER_PID *row = &tcp->table[i];
        /* The listener class selects listeners; the reference build (26300)
         * misreports their dwState (0, 1, FIN_WAIT_1), so it is not checked. */
        if ((u_short)row->dwLocalPort == local.sin_port && row->dwOwningPid == me)
            found = 1;
    }
    /* A buffer too small reports the size it needs. */
    DWORD small = 4;
    DWORD too_small = GetExtendedTcpTable(table, &small, TRUE, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
    long listener_failures = !listening | !found << 1 | (too_small != ERROR_INSUFFICIENT_BUFFER) << 2 |
                             (small < 4 + sizeof(MIB_TCPROW_OWNER_PID)) << 3 | (r != NO_ERROR) << 4;
    step(3, "tcp-listener", listener_failures == 0, listener_failures);

    /* 4. A bound UDP socket in the owner UDP table. */
    SOCKET udp = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    struct sockaddr_in u;
    ZeroMemory(&u, sizeof u);
    u.sin_family = AF_INET;
    size = sizeof u;
    int bound = bind(udp, (struct sockaddr *)&u, sizeof u) == 0 && getsockname(udp, (struct sockaddr *)&u, &size) == 0;
    bytes = sizeof table;
    r = GetExtendedUdpTable(table, &bytes, TRUE, AF_INET, UDP_TABLE_OWNER_PID, 0);
    PMIB_UDPTABLE_OWNER_PID udps = (PMIB_UDPTABLE_OWNER_PID)table;
    found = 0;
    for (DWORD i = 0; r == NO_ERROR && i < udps->dwNumEntries; i++) {
        if ((u_short)udps->table[i].dwLocalPort == u.sin_port && udps->table[i].dwOwningPid == me) found = 1;
    }
    step(4, "udp-table", bound && found, (long)r);

    /* 5. A connection to the echo server in GetTcpTable2, established. */
    SOCKET client = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in server;
    ZeroMemory(&server, sizeof server);
    server.sin_family = AF_INET;
    server.sin_port = htons((u_short)tcp_port);
    server.sin_addr = target;
    int connected = connect(client, (struct sockaddr *)&server, sizeof server) == 0;
    bytes = sizeof table;
    r = GetTcpTable2((PMIB_TCPTABLE2)table, &bytes, TRUE);
    PMIB_TCPTABLE2 tcp2 = (PMIB_TCPTABLE2)table;
    found = 0;
    for (DWORD i = 0; r == NO_ERROR && i < tcp2->dwNumEntries; i++) {
        MIB_TCPROW2 *row = &tcp2->table[i];
        if ((u_short)row->dwRemotePort == server.sin_port && row->dwRemoteAddr == target.S_un.S_addr &&
            row->dwState == MIB_TCP_STATE_ESTAB && row->dwOwningPid == me)
            found = 1;
    }
    step(5, "tcp-connection", connected && found, (long)r);

    /* 6. Protocol statistics: Windows' fixed fields, traffic counted (the
     *    echo of step 1 when it left the machine, the open connection),
     *    and the errors for a missing buffer or another family. */
    MIB_IPSTATS ips;
    MIB_TCPSTATS tcps;
    MIB_UDPSTATS udps_;
    MIB_ICMP icmps;
    MIB_ICMP_EX icmp6;
    int remote = target.S_un.S_addr != htonl(INADDR_LOOPBACK);
    long stats_failures =
        (GetIpStatistics(&ips) != NO_ERROR || ips.dwDefaultTTL != 128 || ips.dwInReceives == 0 ||
         ips.dwOutRequests == 0 || ips.dwNumIf < 2) |
        (GetTcpStatistics(&tcps) != NO_ERROR || tcps.dwRtoAlgorithm != MIB_TCP_RTO_VANJ ||
         tcps.dwMaxConn != MIB_TCP_MAXCONN_DYNAMIC || tcps.dwActiveOpens == 0 || tcps.dwCurrEstab == 0 ||
         tcps.dwInSegs == 0) << 1 |
        (GetUdpStatistics(&udps_) != NO_ERROR || udps_.dwOutDatagrams == 0) << 2 |
        (GetIcmpStatistics(&icmps) != NO_ERROR ||
         (remote && (icmps.stats.icmpOutStats.dwEchos == 0 || icmps.stats.icmpInStats.dwEchoReps == 0))) << 3 |
        (GetIcmpStatisticsEx(&icmp6, AF_INET6) != NO_ERROR) << 4 |
        (GetTcpStatisticsEx(&tcps, 99) != ERROR_NOT_SUPPORTED) << 5 |
        (GetIpStatistics(NULL) != ERROR_INVALID_PARAMETER) << 6;
    step(6, "statistics", stats_failures == 0, stats_failures);

    /* 7. The ARP table: the default gateway is a dynamic neighbour with an
     *    Ethernet address, in GetIpNetTable and GetIpNetTable2 alike. */
    DWORD gateway = 0;
    bytes = sizeof table;
    if (GetIpForwardTable((PMIB_IPFORWARDTABLE)table, &bytes, TRUE) == NO_ERROR) {
        PMIB_IPFORWARDTABLE routes = (PMIB_IPFORWARDTABLE)table;
        for (DWORD i = 0; i < routes->dwNumEntries && !gateway; i++)
            if (routes->table[i].dwForwardDest == 0 && routes->table[i].dwForwardMask == 0)
                gateway = routes->table[i].dwForwardNextHop;
    }
    ULONG needed = 0;
    r = GetIpNetTable(NULL, &needed, TRUE);
    int sized = r == ERROR_INSUFFICIENT_BUFFER && needed >= sizeof(MIB_IPNETTABLE);
    bytes = sizeof table;
    r = GetIpNetTable((PMIB_IPNETTABLE)table, &bytes, TRUE);
    PMIB_IPNETTABLE arp = (PMIB_IPNETTABLE)table;
    int in_table = 0;
    for (DWORD i = 0; r == NO_ERROR && i < arp->dwNumEntries; i++) {
        MIB_IPNETROW *row = &arp->table[i];
        if (row->dwAddr == gateway && row->dwPhysAddrLen == 6 && row->dwType == MIB_IPNET_TYPE_DYNAMIC) in_table = 1;
    }
    PMIB_IPNET_TABLE2 arp2 = NULL;
    int in_table2 = 0;
    if (GetIpNetTable2(AF_INET, &arp2) == NO_ERROR) {
        for (ULONG i = 0; i < arp2->NumEntries; i++) {
            MIB_IPNET_ROW2 *row = &arp2->Table[i];
            if (row->Address.si_family == AF_INET && row->Address.Ipv4.sin_addr.S_un.S_addr == gateway &&
                row->PhysicalAddressLength == 6 && !row->IsUnreachable)
                in_table2 = 1;
        }
        FreeMibTable(arp2);
    }
    long arp_failures = !gateway | !sized << 1 | !in_table << 2 | !in_table2 << 3;
    step(7, "arp-table", arp_failures == 0, arp_failures);
    closesocket(client);
    closesocket(udp);
    closesocket(listener);
    WSACleanup();
    put(failed_step ? "DIAG-A FAILED\r\n" : "DIAG-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

/* CONNECT-A loopback, multicast and raw-socket probe (P15): an ordinary
 * Windows console program (Windows SDK, no C runtime):
 *
 *   mcastprobe.exe <echo host>
 *
 * TCP and UDP over the loopback interface (127.0.0.1, ::1), the IP-level
 * socket options and their defaults, multicast groups joined on the
 * loopback interface (delivery, IP_MULTICAST_LOOP on the receiver, leaving),
 * IPv6 multicast, and a raw ICMP socket: refused with WSAEACCES unless the
 * process is elevated, else an echo to <echo host> whose reply arrives with
 * its IP header. Everything stays on the loopback interface except that
 * echo. One "MC-A <step> ..." line per step ("MC-A note" lines carry
 * measured values); exit 0 when every step passed, else the number of the
 * first failed one. */
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
static int same_bytes(const void *a, const void *b, int n) {
    for (int i = 0; i < n; i++)
        if (((const unsigned char *)a)[i] != ((const unsigned char *)b)[i]) return 0;
    return 1;
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("MC-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static void note(const char *name, long value) {
    put("MC-A note ");
    put(name);
    put("=");
    number(value);
    put("\r\n");
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
/* An integer socket option, or -1000 - the error. */
static long option(SOCKET s, int level, int name) {
    DWORD v = 0;
    int size = sizeof v;
    if (getsockopt(s, level, name, (char *)&v, &size) != 0) return -1000 - WSAGetLastError();
    return (long)v;
}
/* setsockopt with a DWORD: 0, or the error. */
static int set(SOCKET s, int level, int name, DWORD v) {
    return setsockopt(s, level, name, (const char *)&v, sizeof v) == 0 ? 0 : WSAGetLastError();
}
static void timeout(SOCKET s, DWORD ms) {
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char *)&ms, sizeof ms);
}
static SOCKADDR_IN v4(unsigned long address, int port) {
    SOCKADDR_IN a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = address;
    a.sin_port = htons((u_short)port);
    return a;
}
static SOCKADDR_IN6 v6(const IN6_ADDR *address, int port) {
    SOCKADDR_IN6 a;
    memset(&a, 0, sizeof a);
    a.sin6_family = AF_INET6;
    a.sin6_addr = *address;
    a.sin6_port = htons((u_short)port);
    return a;
}
/* The port a bound socket has. */
static int port_of(SOCKET s) {
    SOCKADDR_STORAGE a;
    int size = sizeof a;
    if (getsockname(s, (struct sockaddr *)&a, &size) != 0) return 0;
    return ntohs(((SOCKADDR_IN *)&a)->sin_port);
}
/* Listen on `server` (port 0: any), connect, accept, echo both ways. */
static int tcp_round_trip(int family, const struct sockaddr *server, int size) {
    SOCKET listener = socket(family, SOCK_STREAM, IPPROTO_TCP);
    if (listener == INVALID_SOCKET || bind(listener, server, size) != 0 || listen(listener, 1) != 0) return 0;
    SOCKADDR_STORAGE target;
    int target_size = sizeof target;
    getsockname(listener, (struct sockaddr *)&target, &target_size);
    SOCKET client = socket(family, SOCK_STREAM, IPPROTO_TCP);
    int ok = 0;
    if (client != INVALID_SOCKET && connect(client, (struct sockaddr *)&target, target_size) == 0) {
        SOCKADDR_STORAGE peer;
        int peer_size = sizeof peer;
        SOCKET accepted = accept(listener, (struct sockaddr *)&peer, &peer_size);
        char back[8];
        if (accepted != INVALID_SOCKET) {
            timeout(accepted, 2000);
            timeout(client, 2000);
            ok = send(client, "loopback", 8, 0) == 8 && recv(accepted, back, 8, MSG_WAITALL) == 8 &&
                 same_bytes(back, "loopback", 8) && send(accepted, "returned", 8, 0) == 8 &&
                 recv(client, back, 8, MSG_WAITALL) == 8 && same_bytes(back, "returned", 8);
            closesocket(accepted);
        }
    }
    if (client != INVALID_SOCKET) closesocket(client);
    closesocket(listener);
    return ok;
}
/* One datagram `data` from `sender` to `to`, read on `receiver`: 1 when it
 * arrives from `from` (16 bytes of IPv6, or 4 of IPv4) and the sender's port. */
static int datagram(SOCKET sender, const struct sockaddr *to, int to_size, SOCKET receiver, const void *from,
                    int from_size) {
    char buffer[64];
    SOCKADDR_STORAGE source;
    int source_size = sizeof source;
    if (sendto(sender, "doors-datagram", 14, 0, to, to_size) != 14) return 0;
    int n = recvfrom(receiver, buffer, sizeof buffer, 0, (struct sockaddr *)&source, &source_size);
    if (n != 14 || !same_bytes(buffer, "doors-datagram", 14)) return 0;
    int sender_port = port_of(sender);
    if (source.ss_family == AF_INET6) {
        SOCKADDR_IN6 *s = (SOCKADDR_IN6 *)&source;
        return from_size == 16 && same_bytes(&s->sin6_addr, from, 16) && ntohs(s->sin6_port) == sender_port;
    }
    SOCKADDR_IN *s = (SOCKADDR_IN *)&source;
    return from_size == 4 && same_bytes(&s->sin_addr, from, 4) && ntohs(s->sin_port) == sender_port;
}
/* Nothing arrives within the receive timeout. */
static int silent(SOCKET sender, const struct sockaddr *to, int to_size, SOCKET receiver) {
    char buffer[64];
    if (sendto(sender, "unwanted", 8, 0, to, to_size) != 8) return 0;
    return recv(receiver, buffer, sizeof buffer, 0) == SOCKET_ERROR && WSAGetLastError() == WSAETIMEDOUT;
}
static int elevated(void) {
    HANDLE token;
    TOKEN_ELEVATION e;
    DWORD size = 0;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return 0;
    int r = GetTokenInformation(token, TokenElevation, &e, sizeof e, &size) && e.TokenIsElevated;
    CloseHandle(token);
    return r;
}
static unsigned short checksum(const unsigned char *data, int n) {
    unsigned long sum = 0;
    for (int i = 0; i + 1 < n; i += 2) sum += (unsigned long)(data[i] << 8 | data[i + 1]);
    if (n & 1) sum += (unsigned long)data[n - 1] << 8;
    while (sum >> 16) sum = (sum & 0xffff) + (sum >> 16);
    return (unsigned short)~sum;
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
    if (arguments(line, argv) < 2) {
        put("usage: mcastprobe echo-host\r\n");
        ExitProcess(100);
    }
    IN_ADDR echo_host;
    inet_pton(AF_INET, argv[1], &echo_host);
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
    const unsigned long loopback = htonl(INADDR_LOOPBACK);
    IN6_ADDR one;
    memset(&one, 0, sizeof one);
    one.s6_addr[15] = 1;

    /* 1. TCP over the loopback interface: 127.0.0.1 and ::1. */
    SOCKADDR_IN local4 = v4(loopback, 0);
    SOCKADDR_IN6 local6 = v6(&one, 0);
    int tcp4 = tcp_round_trip(AF_INET, (struct sockaddr *)&local4, sizeof local4);
    int tcp6 = tcp_round_trip(AF_INET6, (struct sockaddr *)&local6, sizeof local6);
    step(1, "loopback-tcp", tcp4 && tcp6, tcp4 | tcp6 << 1);

    /* 2. UDP over the loopback interface, from 127.0.0.1. */
    SOCKET receiver = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    SOCKET sender = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    int bound = bind(receiver, (struct sockaddr *)&local4, sizeof local4) == 0 &&
                bind(sender, (struct sockaddr *)&local4, sizeof local4) == 0;
    timeout(receiver, 1000);
    SOCKADDR_IN to = v4(loopback, port_of(receiver));
    int udp = bound && datagram(sender, (struct sockaddr *)&to, sizeof to, receiver, &loopback, 4);
    step(2, "loopback-udp", udp, bound | udp << 1);

    /* 3. IP-level options start as Windows' defaults. */
    SOCKET udp6 = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
    long ttl = option(receiver, IPPROTO_IP, IP_TTL);
    long multicast_ttl = option(receiver, IPPROTO_IP, IP_MULTICAST_TTL);
    long multicast_loop = option(receiver, IPPROTO_IP, IP_MULTICAST_LOOP);
    long hops = option(udp6, IPPROTO_IPV6, IPV6_UNICAST_HOPS);
    long multicast_hops = option(udp6, IPPROTO_IPV6, IPV6_MULTICAST_HOPS);
    long multicast_loop6 = option(udp6, IPPROTO_IPV6, IPV6_MULTICAST_LOOP);
    long header = option(receiver, IPPROTO_IP, IP_HDRINCL);
    note("ip_ttl", ttl);
    note("ip_multicast_ttl", multicast_ttl);
    note("ip_multicast_loop", multicast_loop);
    note("ipv6_unicast_hops", hops);
    note("ipv6_multicast_hops", multicast_hops);
    note("ipv6_multicast_loop", multicast_loop6);
    note("udp_ip_hdrincl", header);
    step(3, "defaults",
         ttl == 128 && multicast_ttl == 1 && multicast_loop == 1 && hops == 128 && multicast_hops == 1 &&
             multicast_loop6 == 1 && header == -1000 - WSAEINVAL,
         ttl);

    /* 4. Setting them: the TTL reads back, zero is refused. */
    int set_ttl = set(receiver, IPPROTO_IP, IP_TTL, 64);
    long read_ttl = option(receiver, IPPROTO_IP, IP_TTL);
    int zero_ttl = set(receiver, IPPROTO_IP, IP_TTL, 0);
    long read_zero = option(receiver, IPPROTO_IP, IP_TTL);
    int big_ttl = set(receiver, IPPROTO_IP, IP_TTL, 256);
    long read_big = option(receiver, IPPROTO_IP, IP_TTL);
    int multicast_zero = set(sender, IPPROTO_IP, IP_MULTICAST_TTL, 0);
    int multicast_big = set(sender, IPPROTO_IP, IP_MULTICAST_TTL, 256);
    int hops_default = set(udp6, IPPROTO_IPV6, IPV6_UNICAST_HOPS, (DWORD)-1);
    long read_hops = option(udp6, IPPROTO_IPV6, IPV6_UNICAST_HOPS);
    note("ip_ttl_zero", zero_ttl);
    note("ip_ttl_zero_read", read_zero);
    note("ip_ttl_256", big_ttl);
    note("ip_ttl_256_read", read_big);
    note("ip_multicast_ttl_zero", multicast_zero);
    note("ip_multicast_ttl_256", multicast_big);
    note("ipv6_hops_minus_one", hops_default);
    note("ipv6_hops_minus_one_read", read_hops);
    step(4, "set-options",
         set_ttl == 0 && read_ttl == 64 && zero_ttl == 0 && read_zero == 0 && big_ttl == WSAEINVAL && read_big == 0 &&
             multicast_zero == 0 && multicast_big == WSAEINVAL && hops_default == 0 && read_hops == 255,
         read_ttl);
    set(receiver, IPPROTO_IP, IP_TTL, 128);
    set(sender, IPPROTO_IP, IP_MULTICAST_TTL, 1);

    /* 5. Joining a group on the loopback interface; what Windows refuses. */
    IN_ADDR group;
    inet_pton(AF_INET, "239.255.77.1", &group);
    SOCKET member = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    timeout(member, 700);
    IP_MREQ request;
    request.imr_multiaddr = group;
    request.imr_interface.s_addr = loopback;
    int unbound_join = setsockopt(member, IPPROTO_IP, IP_ADD_MEMBERSHIP, (const char *)&request, sizeof request) == 0
                           ? 0
                           : WSAGetLastError();
    note("join_unbound", unbound_join);
    if (unbound_join == 0)
        setsockopt(member, IPPROTO_IP, IP_DROP_MEMBERSHIP, (const char *)&request, sizeof request);
    int joined = bind(member, (struct sockaddr *)&local4, sizeof local4) == 0 &&
                 setsockopt(member, IPPROTO_IP, IP_ADD_MEMBERSHIP, (const char *)&request, sizeof request) == 0;
    int twice = setsockopt(member, IPPROTO_IP, IP_ADD_MEMBERSHIP, (const char *)&request, sizeof request) == 0
                    ? 0
                    : WSAGetLastError();
    IP_MREQ unicast = request;
    inet_pton(AF_INET, "10.1.2.3", &unicast.imr_multiaddr);
    int not_group = setsockopt(member, IPPROTO_IP, IP_ADD_MEMBERSHIP, (const char *)&unicast, sizeof unicast) == 0
                        ? 0
                        : WSAGetLastError();
    SOCKET stream = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    int tcp_join = setsockopt(stream, IPPROTO_IP, IP_ADD_MEMBERSHIP, (const char *)&request, sizeof request) == 0
                       ? 0
                       : WSAGetLastError();
    closesocket(stream);
    note("join_twice", twice);
    note("join_unicast", not_group);
    note("join_tcp", tcp_join);
    step(5, "join",
         unbound_join == 0 && joined && twice == WSAEINVAL && not_group == WSAEADDRNOTAVAIL && tcp_join == WSAEINVAL,
         joined);

    /* 6. This host's datagram to the group arrives from the interface; the
     *    receiver's IP_MULTICAST_LOOP off keeps the next one out. */
    set(sender, IPPROTO_IP, IP_MULTICAST_IF, loopback);
    SOCKADDR_IN group_to = v4(group.s_addr, port_of(member));
    int delivered = datagram(sender, (struct sockaddr *)&group_to, sizeof group_to, member, &loopback, 4);
    int loop_off = set(member, IPPROTO_IP, IP_MULTICAST_LOOP, 0) == 0 &&
                   option(member, IPPROTO_IP, IP_MULTICAST_LOOP) == 0;
    int kept_out = loop_off && silent(sender, (struct sockaddr *)&group_to, sizeof group_to, member);
    set(member, IPPROTO_IP, IP_MULTICAST_LOOP, 1);
    step(6, "group-delivery", delivered && kept_out, delivered | loop_off << 1 | kept_out << 2);

    /* 7. A group left no longer arrives; leaving twice is refused. */
    int left = setsockopt(member, IPPROTO_IP, IP_DROP_MEMBERSHIP, (const char *)&request, sizeof request) == 0;
    int quiet = left && silent(sender, (struct sockaddr *)&group_to, sizeof group_to, member);
    int again = setsockopt(member, IPPROTO_IP, IP_DROP_MEMBERSHIP, (const char *)&request, sizeof request) == 0
                    ? 0
                    : WSAGetLastError();
    note("leave_twice", again);
    step(7, "leave", left && quiet && again == WSAEADDRNOTAVAIL, left | quiet << 1);

    /* 8. IPv6: a group joined on the loopback interface (index 1). */
    IN6_ADDR group6;
    inet_pton(AF_INET6, "ff15::4d:1", &group6);
    SOCKET member6 = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
    SOCKET sender6 = socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
    timeout(member6, 1000);
    IPV6_MREQ request6;
    request6.ipv6mr_multiaddr = group6;
    request6.ipv6mr_interface = 1;
    int joined6 = bind(member6, (struct sockaddr *)&local6, sizeof local6) == 0 &&
                  bind(sender6, (struct sockaddr *)&local6, sizeof local6) == 0 &&
                  setsockopt(member6, IPPROTO_IPV6, IPV6_JOIN_GROUP, (const char *)&request6, sizeof request6) == 0 &&
                  set(sender6, IPPROTO_IPV6, IPV6_MULTICAST_IF, 1) == 0;
    SOCKADDR_IN6 group6_to = v6(&group6, port_of(member6));
    int delivered6 = joined6 && datagram(sender6, (struct sockaddr *)&group6_to, sizeof group6_to, member6, &one, 16);
    int left6 =
        setsockopt(member6, IPPROTO_IPV6, IPV6_LEAVE_GROUP, (const char *)&request6, sizeof request6) == 0;
    step(8, "ipv6-group", joined6 && delivered6 && left6, joined6 | delivered6 << 1 | left6 << 2);

    /* 9. Raw sockets: ICMP and ICMPv6 for every user (Windows' measured
     *    rule), other protocols for administrators (elevated) only. An echo
     *    request to the echo host; the reply arrives with its IPv4 header. */
    int admin = elevated();
    SOCKET raw = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    int raw_error = raw == INVALID_SOCKET ? WSAGetLastError() : 0;
    note("elevated", admin);
    note("raw_error", raw_error);
    SOCKET icmp6 = socket(AF_INET6, SOCK_RAW, IPPROTO_ICMPV6);
    int icmp6_error = icmp6 == INVALID_SOCKET ? WSAGetLastError() : 0;
    note("raw_icmp6", icmp6_error);
    if (icmp6 != INVALID_SOCKET) closesocket(icmp6);
    SOCKET igmp = socket(AF_INET, SOCK_RAW, IPPROTO_IGMP);
    int igmp_error = igmp == INVALID_SOCKET ? WSAGetLastError() : 0;
    note("raw_igmp", igmp_error);
    if (igmp != INVALID_SOCKET) closesocket(igmp);
    int rights = icmp6_error == 0 && igmp_error == (admin ? 0 : WSAEACCES);
    int raw_ok = 0;
    if (raw != INVALID_SOCKET) {
        long kind = option(raw, SOL_SOCKET, SO_TYPE);
        long included = option(raw, IPPROTO_IP, IP_HDRINCL);
        unsigned char request_bytes[16];
        memset(request_bytes, 0, sizeof request_bytes);
        request_bytes[0] = 8; /* echo request */
        request_bytes[4] = 0x4d;
        request_bytes[5] = 0x43; /* identifier "MC" */
        request_bytes[7] = 1;    /* sequence 1 */
        for (int i = 8; i < 16; i++) request_bytes[i] = (unsigned char)('a' + i);
        unsigned short sum = checksum(request_bytes, sizeof request_bytes);
        request_bytes[2] = (unsigned char)(sum >> 8);
        request_bytes[3] = (unsigned char)sum;
        SOCKADDR_IN target = v4(echo_host.s_addr, 0);
        timeout(raw, 2000);
        int sent = sendto(raw, (const char *)request_bytes, sizeof request_bytes, 0, (struct sockaddr *)&target,
                          sizeof target) == sizeof request_bytes;
        int replied = 0;
        for (int tries = 0; sent && tries < 8 && !replied; tries++) {
            unsigned char packet[256];
            SOCKADDR_IN from;
            int from_size = sizeof from;
            int n = recvfrom(raw, (char *)packet, sizeof packet, 0, (struct sockaddr *)&from, &from_size);
            if (n <= 0) break;
            int header_bytes = (packet[0] & 0x0f) * 4;
            /* An echo reply with the identifier, from the host asked. */
            if (n >= header_bytes + 16 && packet[0] >> 4 == 4 && packet[9] == IPPROTO_ICMP &&
                packet[header_bytes] == 0 && packet[header_bytes + 4] == 0x4d && packet[header_bytes + 5] == 0x43 &&
                same_bytes(packet + header_bytes + 8, request_bytes + 8, 8) &&
                from.sin_addr.s_addr == echo_host.s_addr)
                replied = 1;
        }
        note("raw_type", kind);
        note("raw_hdrincl", included);
        raw_ok = kind == SOCK_RAW && included == 0 && sent && replied;
    }
    if (raw != INVALID_SOCKET) closesocket(raw);
    step(9, "raw-icmp", raw_ok && rights, admin * 1000 + raw_ok + rights * 2);

    WSACleanup();
    put(failed_step ? "MC-A FAILED\r\n" : "MC-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

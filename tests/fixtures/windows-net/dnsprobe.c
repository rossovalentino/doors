/* CONNECT-A DNS and IP Helper probe (P15): an ordinary Windows console
 * program (Windows SDK, no C runtime) that reads the adapter through
 * iphlpapi and resolves names through dnsapi against the test DNS server
 * the acceptance run starts on this host:
 *
 *   dnsprobe.exe <dns server port> [server address, default 10.0.2.2]
 *
 * One "NAMES-A <step> ..." line per check; exit 0 only when all passed. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <iphlpapi.h>
#include <windns.h>
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
static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}
static int failed_step;
static void step(int id, const char *name, int ok, long detail) {
    put("NAMES-A ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
static unsigned char ip_byte(DWORD address, int i) {
    return (unsigned char)(address >> (8 * i));
}
static int is(DWORD address, int a, int b, int c, int d) {
    return ip_byte(address, 0) == a && ip_byte(address, 1) == b && ip_byte(address, 2) == c && ip_byte(address, 3) == d;
}
static int sa_is(SOCKET_ADDRESS *sa, int a, int b, int c, int d) {
    return sa->lpSockaddr && sa->lpSockaddr->sa_family == AF_INET &&
        is(((struct sockaddr_in *)sa->lpSockaddr)->sin_addr.s_addr, a, b, c, d);
}

/* DnsQueryEx against the test server at 10.0.2.2:port. */
static DNS_ADDR_ARRAY servers;
static DNS_STATUS query(const wchar_t *name, WORD type, DNS_RECORD **records) {
    DNS_QUERY_REQUEST request;
    DNS_QUERY_RESULT result;
    memset(&request, 0, sizeof request);
    memset(&result, 0, sizeof result);
    request.Version = DNS_QUERY_REQUEST_VERSION1;
    request.QueryName = name;
    request.QueryType = type;
    request.QueryOptions = DNS_QUERY_BYPASS_CACHE;
    request.pDnsServerList = &servers;
    result.Version = DNS_QUERY_REQUEST_VERSION1;
    DNS_STATUS status = DnsQueryEx(&request, &result, NULL);
    *records = result.pQueryRecords;
    return status;
}
static volatile LONG callback_status = -1;
static volatile LONG callback_records;
static VOID WINAPI completed(PVOID context, PDNS_QUERY_RESULT result) {
    DNS_RECORD *r;
    LONG n = 0;
    for (r = result->pQueryRecords; r; r = r->pNext) n++;
    if (result->pQueryRecords) DnsRecordListFree(result->pQueryRecords, DnsFreeRecordList);
    callback_records = n + (context == (PVOID)&servers ? 0 : 1000);
    InterlockedExchange(&callback_status, (LONG)result->QueryStatus);
}
static volatile LONG notifications = -1;
static VOID WINAPI changed(PVOID context, PMIB_IPINTERFACE_ROW row, MIB_NOTIFICATION_TYPE type) {
    (void)context;
    if (type == MibInitialNotification && row == NULL) InterlockedExchange(&notifications, 1);
}

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    int port = 0;
    /* The server address (default 10.0.2.2, the host as the guest sees it). */
    unsigned long server = 0x0202000a;
    {
        LPWSTR line = GetCommandLineW();
        while (*line && *line != ' ') line++;
        while (*line == ' ') line++;
        while (*line >= '0' && *line <= '9') port = port * 10 + (*line++ - '0');
        while (*line == ' ') line++;
        if (*line) {
            unsigned long value = 0, part = 0;
            int shift = 0;
            for (; *line; line++) {
                if (*line == '.') {
                    value |= part << shift;
                    shift += 8;
                    part = 0;
                } else {
                    part = part * 10 + (unsigned long)(*line - '0');
                }
            }
            server = value | part << shift;
        }
    }
    if (!port) {
        put("usage: dnsprobe <dns port>\r\n");
        ExitProcess(100);
    }

    /* 1. The adapter: Ethernet with the DHCP lease, then the loopback. */
    static char buffer[16384];
    ULONG size = sizeof buffer;
    ULONG r = GetAdaptersAddresses(AF_UNSPEC, GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_INCLUDE_PREFIX, NULL,
                                   (IP_ADAPTER_ADDRESSES *)buffer, &size);
    IP_ADAPTER_ADDRESSES *a = (IP_ADAPTER_ADDRESSES *)buffer;
    /* With IPv6 on the adapter Windows lists its addresses, gateway and
       DNS servers ahead of IPv4's: look for the IPv4 entries. */
    IP_ADAPTER_UNICAST_ADDRESS *unicast = r == NO_ERROR ? a->FirstUnicastAddress : NULL;
    while (unicast && !sa_is(&unicast->Address, 10, 0, 2, 15)) unicast = unicast->Next;
    IP_ADAPTER_GATEWAY_ADDRESS_LH *gateway = r == NO_ERROR ? a->FirstGatewayAddress : NULL;
    while (gateway && !sa_is(&gateway->Address, 10, 0, 2, 2)) gateway = gateway->Next;
    IP_ADAPTER_DNS_SERVER_ADDRESS *dns = r == NO_ERROR ? a->FirstDnsServerAddress : NULL;
    while (dns && !sa_is(&dns->Address, 10, 0, 2, 3)) dns = dns->Next;
    int adapter_ok = r == NO_ERROR && a->IfType == IF_TYPE_ETHERNET_CSMACD && wide_equal(a->FriendlyName, L"Ethernet") &&
        a->OperStatus == IfOperStatusUp && unicast && unicast->OnLinkPrefixLength == 24 && gateway && dns &&
        a->PhysicalAddressLength == 6 && a->Mtu == 1500 && a->Next && a->Next->IfType == IF_TYPE_SOFTWARE_LOOPBACK &&
        a->Next->Next == NULL;
    step(1, "adapters", adapter_ok, (long)r);
    ULONG tiny = 16;
    r = GetAdaptersAddresses(AF_INET, 0, NULL, (IP_ADAPTER_ADDRESSES *)buffer, &tiny);
    step(2, "overflow", r == ERROR_BUFFER_OVERFLOW && tiny > 448, (long)tiny);

    /* 3. Interface table, names and the best route. */
    MIB_IF_TABLE2 *table = NULL;
    r = GetIfTable2(&table);
    int rows = r == NO_ERROR ? (int)table->NumEntries : -1;
    NET_LUID luid;
    luid.Value = 0;
    for (int i = 0; i < rows; i++) {
        if (table->Table[i].Type == IF_TYPE_ETHERNET_CSMACD) luid = table->Table[i].InterfaceLuid;
    }
    if (table) FreeMibTable(table);
    wchar_t name[64];
    DWORD best = 0;
    int names_ok = ConvertInterfaceLuidToNameW(&luid, name, 64) == NO_ERROR && wide_equal(name, L"ethernet_32768") &&
        GetBestInterface(0x08080808, &best) == NO_ERROR;
    NET_IFINDEX index = 0;
    names_ok = names_ok && ConvertInterfaceLuidToIndex(&luid, &index) == NO_ERROR && index == best;
    step(3, "interfaces", rows == 2 && names_ok, (long)best);

    /* 4. Network parameters and the resolver's server list. */
    static char fixed[2048];
    size = sizeof fixed;
    r = GetNetworkParams((FIXED_INFO *)fixed, &size);
    char servers_buffer[64];
    DWORD servers_size = sizeof servers_buffer;
    DNS_STATUS config = DnsQueryConfig(DnsConfigDnsServerList, 0, NULL, NULL, servers_buffer, &servers_size);
    IP4_ARRAY *list = (IP4_ARRAY *)servers_buffer;
    step(4, "parameters", r == NO_ERROR && ((FIXED_INFO *)fixed)->HostName[0] && config == 0 && list->AddrCount >= 1 &&
         is(list->AddrArray[0], 10, 0, 2, 3), (long)config);

    /* 5. A change subscription gets its initial notification. */
    HANDLE notification = NULL;
    r = NotifyIpInterfaceChange(AF_UNSPEC, changed, NULL, TRUE, &notification);
    int notified = notifications == 1;
    if (notification) CancelMibChangeNotify2(notification);
    step(5, "notify", r == NO_ERROR && notified, (long)r);

    /* The test server: 10.0.2.2:port. */
    memset(&servers, 0, sizeof servers);
    servers.MaxCount = 1;
    servers.AddrCount = 1;
    servers.Family = AF_INET;
    struct sockaddr_in *sa = (struct sockaddr_in *)servers.AddrArray[0].MaxSa;
    sa->sin_family = AF_INET;
    sa->sin_port = (u_short)(((port & 0xff) << 8) | (port >> 8));
    sa->sin_addr.s_addr = server;

    /* 6. A records through a CNAME. */
    DNS_RECORD *records = NULL;
    DNS_STATUS status = query(L"www.doors.test", DNS_TYPE_A, &records);
    int cname = records && records->wType == DNS_TYPE_CNAME && wide_equal(records->Data.CNAME.pNameHost, L"host.doors.test") &&
        records->pNext && records->pNext->wType == DNS_TYPE_A && is(records->pNext->Data.A.IpAddress, 10, 9, 8, 7) &&
        (records->Flags.DW & 3) == DnsSectionAnswer;
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    step(6, "a-cname", status == 0 && cname, (long)status);

    /* 7. MX and SRV. */
    status = query(L"doors.test", DNS_TYPE_MX, &records);
    int mx = records && records->wType == DNS_TYPE_MX && records->Data.MX.wPreference == 10 &&
        wide_equal(records->Data.MX.pNameExchange, L"mail.doors.test");
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    DNS_STATUS srv_status = query(L"_probe._tcp.doors.test", DNS_TYPE_SRV, &records);
    int srv = records && records->wType == DNS_TYPE_SRV && records->Data.SRV.wPort == 8080 &&
        records->Data.SRV.wWeight == 5 && wide_equal(records->Data.SRV.pNameTarget, L"svc.doors.test");
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    step(7, "mx-srv", status == 0 && mx && srv_status == 0 && srv, (long)srv_status);

    /* 8. A TXT answer over 192 bytes by UDP, and one over 512 that the
     * server truncates, fetched again over TCP. */
    status = query(L"medium.doors.test", DNS_TYPE_TEXT, &records);
    DWORD medium_strings = records && records->wType == DNS_TYPE_TEXT ? records->Data.TXT.dwStringCount : 0;
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    DNS_STATUS large_status = query(L"large.doors.test", DNS_TYPE_TEXT, &records);
    DWORD large_strings = records ? records->Data.TXT.dwStringCount : 0;
    int large_tail = records && large_strings == 5 && records->Data.TXT.pStringArray[4][199] == L'z';
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    step(8, "txt-tcp", status == 0 && medium_strings == 2 && large_status == 0 && large_tail, (long)large_strings);

    /* 9. NXDOMAIN, and NODATA with the zone's SOA. */
    status = query(L"missing.doors.test", DNS_TYPE_A, &records);
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    DNS_STATUS nodata = query(L"host.doors.test", DNS_TYPE_AAAA, &records);
    int soa = records && records->wType == DNS_TYPE_SOA && (records->Flags.DW & 3) == DnsSectionAuthority;
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    step(9, "negative", status == DNS_ERROR_RCODE_NAME_ERROR && nodata == DNS_INFO_NO_RECORDS && soa, (long)status);

    /* 10. An asynchronous query completes through its callback. */
    DNS_QUERY_REQUEST request;
    static DNS_QUERY_RESULT result;
    memset(&request, 0, sizeof request);
    request.Version = DNS_QUERY_REQUEST_VERSION1;
    request.QueryName = L"www.doors.test";
    request.QueryType = DNS_TYPE_A;
    request.pDnsServerList = &servers;
    request.pQueryCompletionCallback = completed;
    request.pQueryContext = &servers;
    result.Version = DNS_QUERY_REQUEST_VERSION1;
    DNS_QUERY_CANCEL cancel;
    status = DnsQueryEx(&request, &result, &cancel);
    for (int i = 0; i < 200 && callback_status == -1; i++) Sleep(50);
    step(10, "async", status == DNS_REQUEST_PENDING && callback_status == 0 && callback_records == 2, callback_records);

    /* 11. Local answers and name rules. */
    status = DnsQuery_W(L"localhost", DNS_TYPE_A, DNS_QUERY_STANDARD, NULL, &records, NULL);
    int local = status == 0 && records && is(records->Data.A.IpAddress, 127, 0, 0, 1);
    if (records) DnsRecordListFree(records, DnsFreeRecordList);
    int rules = DnsValidateName_W(L"my_host", DnsNameHostnameFull) == DNS_ERROR_NON_RFC_NAME &&
        DnsValidateName_W(L"a/b", DnsNameDomain) == DNS_ERROR_INVALID_NAME_CHAR &&
        DnsValidateName_W(L"_sip._tcp.example.com", DnsNameSrvRecord) == 0 &&
        DnsNameCompare_W(L"Doors.TEST.", L"doors.test");
    step(11, "local-rules", local && rules, (long)status);

    put(failed_step ? "NAMES-A FAILED\r\n" : "NAMES-A PASSED\r\n");
    ExitProcess((UINT)failed_step);
}

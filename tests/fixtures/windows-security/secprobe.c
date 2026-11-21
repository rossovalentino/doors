/* SECURITY-A probe (P16 M5): an ordinary Windows console program (Windows
 * SDK, no C runtime) that exercises advapi32's token, SID, security
 * descriptor, SDDL, access check, file and registry security and
 * CryptoAPI functions.
 *
 *   secprobe.exe dump   Windows' tables as this system answers them
 *                       (well-known SIDs, SDDL aliases and rights, account
 *                       names, the token, folders' and keys' descriptors);
 *                       reads only
 *
 * One line per record or check. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <sddl.h>
#include <aclapi.h>
#include <wincrypt.h>

#pragma function(memset)
void *memset(void *destination, int value, size_t count) {
    unsigned char *d = destination;
    while (count--) *d++ = (unsigned char)value;
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
    unsigned long u = negative ? (unsigned long)-v : (unsigned long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (negative) b[--i] = '-';
    put(b + i);
}
static void hex(unsigned long v) {
    char b[16];
    int i = 15;
    b[i] = 0;
    do {
        b[--i] = "0123456789abcdef"[v & 15];
        v >>= 4;
    } while (v);
    put("0x");
    put(b + i);
}
static void wput(const wchar_t *s) {
    char b[1024];
    int n = WideCharToMultiByte(CP_UTF8, 0, s, -1, b, sizeof b, NULL, NULL);
    if (n > 0) put(b);
}
static void nl(void) { put("\r\n"); }
static void sid_text(PSID sid) {
    LPWSTR text = NULL;
    if (sid && ConvertSidToStringSidW(sid, &text)) {
        wput(text);
        LocalFree(text);
    } else {
        put("-");
    }
}

/* ---- dump ------------------------------------------------------------------- */
static const char *aliases[] = {
    "AA", "AC", "AN", "AO", "AP", "AS", "AU", "BA", "BG", "BO", "BU", "CA", "CD", "CG",
    "CN", "CO", "CY", "DA", "DC", "DD", "DG", "DU", "EA", "ED", "EK", "ER", "ES", "HA",
    "HI", "HO", "IS", "IU", "KA", "LA", "LG", "LS", "LU", "LW", "ME", "MP", "MS", "MU",
    "NO", "NS", "NU", "OW", "PA", "PO", "PS", "PU", "RA", "RC", "RD", "RE", "RM", "RO",
    "RS", "RU", "SA", "SI", "SO", "SS", "SU", "SY", "UD", "WD", "WR",
};
static const DWORD masks[] = {
    0x10000000, 0x80000000, 0x40000000, 0x20000000, 0xA0000000, 0xE0000000, 0xF0000000,
    0x1f01ff, 0x120089, 0x120116, 0x1200a0, 0x1200a9, 0x1301bf, 0x130197, 0x1201bf,
    0xf003f, 0x20019, 0x20006, 0x2001f, 0x10000, 0x20000, 0x40000, 0x80000, 0x100000,
    0xd0000, 0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x3f, 0x1ff,
    0x1f0000, 0x30000, 0x1, 0x110000, 0x12019f, 0x1f0003, 0x100001, 0x1fffff,
};
static void dump_sid_name(PSID sid) {
    wchar_t name[128], domain[128];
    DWORD n = 128, d = 128;
    SID_NAME_USE use;
    if (LookupAccountSidW(NULL, sid, name, &n, domain, &d, &use)) {
        wput(domain);
        put("\\");
        wput(name);
        put(" ");
        number(use);
    } else {
        put("error ");
        number((long)GetLastError());
    }
}
static void dump_sddl(const wchar_t *label, PSECURITY_DESCRIPTOR sd, SECURITY_INFORMATION which) {
    LPWSTR text = NULL;
    wput(label);
    put(" ");
    if (ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, which, &text, NULL)) {
        wput(text);
        LocalFree(text);
    } else {
        put("error ");
        number((long)GetLastError());
    }
    nl();
}
static void dump_named(const wchar_t *path, SE_OBJECT_TYPE type) {
    PSECURITY_DESCRIPTOR sd = NULL;
    SECURITY_INFORMATION which = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION |
                                 DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    DWORD e = GetNamedSecurityInfoW(path, type, which, NULL, NULL, NULL, NULL, &sd);
    put("named ");
    if (e) {
        wput(path);
        put(" error ");
        number((long)e);
        nl();
        return;
    }
    dump_sddl(path, sd, which);
    LocalFree(sd);
}
static void dump_token(void) {
    HANDLE token;
    static BYTE b[4096];
    DWORD n = 0, i;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
        put("token error ");
        number((long)GetLastError());
        nl();
        return;
    }
    if (GetTokenInformation(token, TokenElevationType, b, sizeof b, &n)) {
        put("token elevation-type ");
        number(*(DWORD *)b);
        nl();
    }
    if (GetTokenInformation(token, TokenElevation, b, sizeof b, &n)) {
        put("token elevation ");
        number(*(DWORD *)b);
        nl();
    }
    put("token linked ");
    if (GetTokenInformation(token, TokenLinkedToken, b, sizeof b, &n)) {
        put("ok");
        CloseHandle(*(HANDLE *)b);
    } else {
        put("error ");
        number((long)GetLastError());
    }
    nl();
    if (GetTokenInformation(token, TokenHasRestrictions, b, sizeof b, &n)) {
        put("token restricted ");
        number(*(DWORD *)b);
        nl();
    }
    if (GetTokenInformation(token, TokenUser, b, sizeof b, &n)) {
        put("token user ");
        sid_text(((TOKEN_USER *)b)->User.Sid);
        nl();
    }
    if (GetTokenInformation(token, TokenOwner, b, sizeof b, &n)) {
        put("token owner ");
        sid_text(((TOKEN_OWNER *)b)->Owner);
        put(" ");
        dump_sid_name(((TOKEN_OWNER *)b)->Owner);
        nl();
    }
    if (GetTokenInformation(token, TokenPrimaryGroup, b, sizeof b, &n)) {
        put("token primary-group ");
        sid_text(((TOKEN_PRIMARY_GROUP *)b)->PrimaryGroup);
        put(" ");
        dump_sid_name(((TOKEN_PRIMARY_GROUP *)b)->PrimaryGroup);
        nl();
    }
    if (GetTokenInformation(token, TokenDefaultDacl, b, sizeof b, &n)) {
        ACL *acl = ((TOKEN_DEFAULT_DACL *)b)->DefaultDacl;
        put("token default-dacl aces=");
        number(acl ? acl->AceCount : -1);
        for (i = 0; acl && i < acl->AceCount; i++) {
            ACCESS_ALLOWED_ACE *ace;
            if (GetAce(acl, i, (void **)&ace)) {
                put(" ");
                number(ace->Header.AceType);
                put("/");
                hex(ace->Mask);
                put("/");
                sid_text(&ace->SidStart);
            }
        }
        nl();
    }
    if (GetTokenInformation(token, TokenGroups, b, sizeof b, &n)) {
        TOKEN_GROUPS *g = (TOKEN_GROUPS *)b;
        for (i = 0; i < g->GroupCount; i++) {
            put("token group ");
            sid_text(g->Groups[i].Sid);
            put(" ");
            hex(g->Groups[i].Attributes);
            nl();
        }
    }
    if (GetTokenInformation(token, TokenPrivileges, b, sizeof b, &n)) {
        TOKEN_PRIVILEGES *p = (TOKEN_PRIVILEGES *)b;
        for (i = 0; i < p->PrivilegeCount; i++) {
            put("token privilege ");
            number((long)p->Privileges[i].Luid.LowPart);
            put(" ");
            hex(p->Privileges[i].Attributes);
            nl();
        }
    }
    if (GetTokenInformation(token, TokenType, b, sizeof b, &n)) {
        put("token type ");
        number(*(DWORD *)b);
        nl();
    }
    put("token impersonation-level ");
    if (GetTokenInformation(token, TokenImpersonationLevel, b, sizeof b, &n)) {
        number(*(DWORD *)b);
    } else {
        put("error ");
        number((long)GetLastError());
    }
    nl();
    if (GetTokenInformation(token, TokenStatistics, b, sizeof b, &n)) {
        TOKEN_STATISTICS *s = (TOKEN_STATISTICS *)b;
        put("token statistics type=");
        number(s->TokenType);
        put(" level=");
        number(s->ImpersonationLevel);
        put(" groups=");
        number((long)s->GroupCount);
        put(" privileges=");
        number((long)s->PrivilegeCount);
        put(" dynamic=");
        number((long)s->DynamicCharged);
        put("/");
        number((long)s->DynamicAvailable);
        nl();
    }
    CloseHandle(token);
}
static void dump(void) {
    BYTE sid[SECURITY_MAX_SID_SIZE];
    DWORD i, n;
    for (i = 0; i <= 130; i++) {
        n = sizeof sid;
        put("wks ");
        number((long)i);
        put(" ");
        if (CreateWellKnownSid((WELL_KNOWN_SID_TYPE)i, NULL, sid, &n)) {
            sid_text(sid);
            put(" ");
            dump_sid_name(sid);
            put(" size=");
            number((long)n);
        } else {
            put("error ");
            number((long)GetLastError());
        }
        nl();
    }
    for (i = 0; i < sizeof aliases / sizeof aliases[0]; i++) {
        wchar_t w[3];
        PSID p = NULL;
        w[0] = (wchar_t)aliases[i][0];
        w[1] = (wchar_t)aliases[i][1];
        w[2] = 0;
        put("alias ");
        put(aliases[i]);
        put(" ");
        if (ConvertStringSidToSidW(w, &p)) {
            sid_text(p);
            put(" ");
            dump_sid_name(p);
            LocalFree(p);
        } else {
            put("error ");
            number((long)GetLastError());
        }
        nl();
    }
    for (i = 0; i < sizeof masks / sizeof masks[0]; i++) {
        wchar_t text[64];
        wchar_t digits[16];
        int k = 0, j;
        DWORD m = masks[i];
        PSECURITY_DESCRIPTOR sd = NULL;
        const wchar_t *head = L"D:(A;;0x";
        do {
            digits[k++] = L"0123456789abcdef"[m & 15];
            m >>= 4;
        } while (m);
        for (j = 0; head[j]; j++) text[j] = head[j];
        while (k) text[j++] = digits[--k];
        text[j++] = L';';
        text[j++] = L';';
        text[j++] = L';';
        text[j++] = L'W';
        text[j++] = L'D';
        text[j++] = L')';
        text[j] = 0;
        put("rights ");
        hex(masks[i]);
        put(" ");
        if (ConvertStringSecurityDescriptorToSecurityDescriptorW(text, SDDL_REVISION_1, &sd, NULL)) {
            dump_sddl(L"", sd, DACL_SECURITY_INFORMATION);
            LocalFree(sd);
        } else {
            put("error ");
            number((long)GetLastError());
            nl();
        }
    }
    {
        static const wchar_t *samples[] = {
            L"O:BAG:SYD:P(A;OICI;FA;;;SY)(A;;0x1200a9;;;BU)S:(ML;;NW;;;ME)",
            L"O:SYG:SYD:AI(A;ID;FA;;;SY)(A;OICIIOID;GA;;;BA)(D;;WD;;;WD)",
            L"D:(A;;GA;;;WD)",
            L"O:BA",
            L"D:NO_ACCESS_CONTROL",
            L"D:",
            L"G:S-1-5-21-1-2-3-513D:(A;;KR;;;AU)(A;CI;KA;;;CO)",
            L"S:(ML;OICINPIO;NWNRNX;;;HI)",
        };
        for (i = 0; i < sizeof samples / sizeof samples[0]; i++) {
            PSECURITY_DESCRIPTOR sd = NULL;
            ULONG size = 0;
            put("sddl ");
            wput(samples[i]);
            put(" ");
            if (ConvertStringSecurityDescriptorToSecurityDescriptorW(samples[i], SDDL_REVISION_1, &sd, &size)) {
                ULONG k;
                put("size=");
                number((long)size);
                put(" bytes=");
                for (k = 0; k < size; k++) {
                    {
                        char two[3];
                        two[0] = "0123456789abcdef"[((BYTE *)sd)[k] >> 4];
                        two[1] = "0123456789abcdef"[((BYTE *)sd)[k] & 15];
                        two[2] = 0;
                        put(two);
                    }
                }
                put(" back=");
                dump_sddl(L"", sd,
                          OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION |
                              DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION);
                LocalFree(sd);
            } else {
                put("error ");
                number((long)GetLastError());
                nl();
            }
        }
    }
    {
        static const wchar_t *names[] = {
            L"Administrators", L"BUILTIN\\Administrators", L"SYSTEM", L"NT AUTHORITY\\SYSTEM",
            L"Everyone", L"Users", L"Authenticated Users", L"INTERACTIVE", L"None",
            L"Guests", L"LOCAL SERVICE", L"NoSuchAccountDoors", L"",
        };
        for (i = 0; i < sizeof names / sizeof names[0]; i++) {
            wchar_t domain[128];
            DWORD s = sizeof sid, d = 128;
            SID_NAME_USE use;
            put("name '");
            wput(names[i]);
            put("' ");
            if (LookupAccountNameW(NULL, names[i], sid, &s, domain, &d, &use)) {
                sid_text(sid);
                put(" ");
                wput(domain);
                put(" ");
                number(use);
            } else {
                put("error ");
                number((long)GetLastError());
            }
            nl();
        }
    }
    dump_token();
    {
        wchar_t self[MAX_PATH];
        DWORD k = GetModuleFileNameW(NULL, self, MAX_PATH);
        while (k && self[k - 1] != L'\\') k--;
        if (k) self[k - 1] = 0;
        dump_named(self, SE_FILE_OBJECT);
    }
    dump_named(L"C:\\Program Files", SE_FILE_OBJECT);
    dump_named(L"C:\\ProgramData", SE_FILE_OBJECT);
    dump_named(L"C:\\Windows", SE_FILE_OBJECT);
    dump_named(L"C:\\Windows\\System32", SE_FILE_OBJECT);
    dump_named(L"C:\\Users", SE_FILE_OBJECT);
    dump_named(L"C:\\Users\\Public", SE_FILE_OBJECT);
    dump_named(L"MACHINE\\SOFTWARE", SE_REGISTRY_KEY);
    dump_named(L"CURRENT_USER\\Software", SE_REGISTRY_KEY);
    dump_named(L"CLASSES_ROOT\\.txt", SE_REGISTRY_KEY);
}

static void result(const char *label, BOOL ok) {
    put(label);
    put(ok ? " ok" : " error ");
    if (!ok) number((long)GetLastError());
    nl();
}
static void hash_dump(HCRYPTPROV prov, ALG_ID alg, const char *label) {
    HCRYPTHASH h = 0;
    BYTE digest[64];
    DWORD n = sizeof digest, k, size = 0, sl = sizeof size;
    put("hash ");
    put(label);
    put(" ");
    if (!CryptCreateHash(prov, alg, 0, 0, &h)) {
        put("create error ");
        hex(GetLastError());
        nl();
        return;
    }
    CryptGetHashParam(h, HP_HASHSIZE, (BYTE *)&size, &sl, 0);
    put("size=");
    number((long)size);
    put(" ");
    if (CryptHashData(h, (const BYTE *)"abc", 3, 0) && CryptGetHashParam(h, HP_HASHVAL, digest, &n, 0)) {
        for (k = 0; k < n; k++) {
            char two[3];
            two[0] = "0123456789abcdef"[digest[k] >> 4];
            two[1] = "0123456789abcdef"[digest[k] & 15];
            two[2] = 0;
            put(two);
        }
        /* Hashing after the value was read fails. */
        put(CryptHashData(h, (const BYTE *)"x", 1, 0) ? " again-ok" : " again-error ");
        if (GetLastError()) hex(GetLastError());
    } else {
        put("error ");
        hex(GetLastError());
    }
    nl();
    CryptDestroyHash(h);
}
static void more(void) {
    static BYTE b[4096];
    HANDLE token = 0, thread = 0, dup = 0;
    DWORD n = 0;
    PSECURITY_DESCRIPTOR sd = NULL;
    BYTE sid[SECURITY_MAX_SID_SIZE];
    DWORD s = sizeof sid;
    BOOL member = 2;
    OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &token);
    /* TokenLinkedToken and its buffer sizes. */
    {
        static const DWORD sizes[] = {4, 8, 16};
        DWORD i;
        for (i = 0; i < 3; i++) {
            n = 0;
            put("linked size=");
            number((long)sizes[i]);
            put(" ");
            if (GetTokenInformation(token, TokenLinkedToken, b, sizes[i], &n)) {
                put("ok type=");
                {
                    DWORD t = 0, tn;
                    GetTokenInformation(*(HANDLE *)b, TokenType, &t, 4, &tn);
                    number((long)t);
                    GetTokenInformation(*(HANDLE *)b, TokenImpersonationLevel, &t, 4, &tn);
                    put(" level=");
                    number((long)t);
                    GetTokenInformation(*(HANDLE *)b, TokenElevationType, &t, 4, &tn);
                    put(" elevation-type=");
                    number((long)t);
                }
                CloseHandle(*(HANDLE *)b);
            } else {
                put("error ");
                number((long)GetLastError());
            }
            put(" needed=");
            number((long)n);
            nl();
        }
    }
    /* Membership with the process token (primary) and with none. */
    CreateWellKnownSid(WinBuiltinUsersSid, NULL, sid, &s);
    put("member primary ");
    if (CheckTokenMembership(token, sid, &member)) number(member);
    else {
        put("error ");
        number((long)GetLastError());
    }
    nl();
    put("member null ");
    if (CheckTokenMembership(NULL, sid, &member)) number(member);
    else {
        put("error ");
        number((long)GetLastError());
    }
    nl();
    s = sizeof sid;
    CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, sid, &s);
    put("member null admins ");
    if (CheckTokenMembership(NULL, sid, &member)) number(member);
    nl();
    /* Duplicates and impersonation. */
    result("duplicate", DuplicateToken(token, SecurityIdentification, &dup));
    if (dup) {
        DWORD t = 0;
        GetTokenInformation(dup, TokenType, &t, 4, &n);
        put("duplicate type=");
        number((long)t);
        GetTokenInformation(dup, TokenImpersonationLevel, &t, 4, &n);
        put(" level=");
        number((long)t);
        nl();
        put("member impersonation ");
        if (CheckTokenMembership(dup, sid, &member)) number(member);
        else {
            put("error ");
            number((long)GetLastError());
        }
        nl();
        CloseHandle(dup);
    }
    result("thread-token before", OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread));
    result("impersonate-self", ImpersonateSelf(SecurityImpersonation));
    if (OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread)) {
        DWORD t = 0;
        GetTokenInformation(thread, TokenType, &t, 4, &n);
        put("thread-token type=");
        number((long)t);
        GetTokenInformation(thread, TokenImpersonationLevel, &t, 4, &n);
        put(" level=");
        number((long)t);
        nl();
        /* AccessCheck: Users read; generic read asked. */
        {
            PSECURITY_DESCRIPTOR check = NULL;
            GENERIC_MAPPING map = {FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_GENERIC_EXECUTE, FILE_ALL_ACCESS};
            PRIVILEGE_SET privileges;
            DWORD plen = sizeof privileges, granted = 0;
            BOOL status = 0;
            static const wchar_t *cases[] = {
                L"O:SYG:SYD:(A;;FR;;;BU)",
                L"O:SYG:SYD:(D;;0x2;;;BA)(A;;FA;;;BU)",
                L"O:SYG:SYD:NO_ACCESS_CONTROL",
                L"O:SYG:SYD:",
                L"D:(A;;FA;;;WD)",
                L"O:SYG:SYD:(A;;GA;;;WD)",
            };
            static const DWORD wants[] = {GENERIC_READ, GENERIC_WRITE, MAXIMUM_ALLOWED, FILE_READ_DATA};
            DWORD c, w;
            for (c = 0; c < 6; c++) {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(cases[c], SDDL_REVISION_1, &check, NULL);
                for (w = 0; w < 4; w++) {
                    DWORD want = wants[w];
                    plen = sizeof privileges;
                    granted = 0;
                    status = 2;
                    put("access ");
                    wput(cases[c]);
                    put(" want=");
                    hex(want);
                    put(" ");
                    if (AccessCheck(check, thread, want, &map, &privileges, &plen, &granted, &status)) {
                        put("status=");
                        number(status);
                        put(" granted=");
                        hex(granted);
                        if (!status) {
                            put(" last=");
                            number((long)GetLastError());
                        }
                    } else {
                        put("error ");
                        number((long)GetLastError());
                    }
                    nl();
                }
                LocalFree(check);
            }
            plen = 4;
            put("access small-privileges ");
            ConvertStringSecurityDescriptorToSecurityDescriptorW(cases[0], SDDL_REVISION_1, &check, NULL);
            if (AccessCheck(check, thread, GENERIC_READ, &map, &privileges, &plen, &granted, &status)) put("ok");
            else {
                put("error ");
                number((long)GetLastError());
            }
            put(" plen=");
            number((long)plen);
            nl();
            put("access primary ");
            if (AccessCheck(check, token, GENERIC_READ, &map, &privileges, &plen, &granted, &status)) put("ok");
            else {
                put("error ");
                number((long)GetLastError());
            }
            nl();
            LocalFree(check);
        }
        CloseHandle(thread);
    }
    result("revert", RevertToSelf());
    result("thread-token after", OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread));
    /* Privileges: enable one held and one not held. */
    {
        TOKEN_PRIVILEGES tp, previous;
        DWORD plen = sizeof previous;
        tp.PrivilegeCount = 1;
        tp.Privileges[0].Luid.LowPart = 19; /* SeShutdownPrivilege */
        tp.Privileges[0].Luid.HighPart = 0;
        tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
        put("adjust shutdown ");
        if (AdjustTokenPrivileges(token, FALSE, &tp, sizeof previous, &previous, &plen)) {
            put("ok last=");
            number((long)GetLastError());
            put(" previous=");
            number((long)previous.PrivilegeCount);
            if (previous.PrivilegeCount) {
                put("/");
                number((long)previous.Privileges[0].Luid.LowPart);
                put("/");
                hex(previous.Privileges[0].Attributes);
            }
        } else {
            put("error ");
            number((long)GetLastError());
        }
        nl();
        if (GetTokenInformation(token, TokenPrivileges, b, sizeof b, &n)) {
            TOKEN_PRIVILEGES *p = (TOKEN_PRIVILEGES *)b;
            DWORD i;
            put("privileges after");
            for (i = 0; i < p->PrivilegeCount; i++) {
                put(" ");
                number((long)p->Privileges[i].Luid.LowPart);
                put("/");
                hex(p->Privileges[i].Attributes);
            }
            nl();
        }
        tp.Privileges[0].Luid.LowPart = 20; /* SeDebugPrivilege */
        put("adjust debug ");
        if (AdjustTokenPrivileges(token, FALSE, &tp, 0, NULL, NULL)) {
            put("ok last=");
            number((long)GetLastError());
        } else {
            put("error ");
            number((long)GetLastError());
        }
        nl();
        tp.Privileges[0].Luid.LowPart = 19;
        tp.Privileges[0].Attributes = 0;
        AdjustTokenPrivileges(token, FALSE, &tp, 0, NULL, NULL);
    }
    /* SDDL errors. */
    {
        static const wchar_t *bad[] = {
            L"O:XX", L"D:(Q;;GA;;;WD)", L"D:(A;;GA;;;WD", L"garbage", L"D:(A;;ZZ;;;WD)",
            L"D:(A;XX;GA;;;WD)", L"O:S-1-5-x", L"D:(A;;GA;;;S-1-5-32-544)", L"",
        };
        DWORD i;
        for (i = 0; i < sizeof bad / sizeof bad[0]; i++) {
            put("sddl-error '");
            wput(bad[i]);
            put("' ");
            if (ConvertStringSecurityDescriptorToSecurityDescriptorW(bad[i], SDDL_REVISION_1, &sd, NULL)) {
                put("ok");
                LocalFree(sd);
            } else {
                put("error ");
                number((long)GetLastError());
            }
            nl();
        }
    }
    /* A process object's descriptor. */
    {
        PSECURITY_DESCRIPTOR psd = NULL;
        DWORD e = GetSecurityInfo(GetCurrentProcess(), SE_KERNEL_OBJECT,
                                  OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION |
                                      DACL_SECURITY_INFORMATION,
                                  NULL, NULL, NULL, NULL, &psd);
        put("process ");
        if (e) {
            put("error ");
            number((long)e);
            nl();
        } else {
            dump_sddl(L"", psd, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION);
            LocalFree(psd);
        }
    }
    /* GetFileSecurity sizes on this program. */
    {
        wchar_t self[MAX_PATH];
        DWORD needed = 0;
        GetModuleFileNameW(NULL, self, MAX_PATH);
        put("file-security small ");
        if (GetFileSecurityW(self, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, b, 8, &needed)) put("ok");
        else {
            put("error ");
            number((long)GetLastError());
        }
        put(" needed=");
        number((long)needed);
        nl();
        put("file-security ");
        if (GetFileSecurityW(self, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, b, sizeof b, &needed)) {
            put("control=");
            hex(((SECURITY_DESCRIPTOR_RELATIVE *)b)->Control);
            put(" ");
            dump_sddl(L"", b, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION);
        } else {
            put("error ");
            number((long)GetLastError());
            nl();
        }
        {
            DWORD e = GetNamedSecurityInfoW(L"C:\\no\\such\\file.txt", SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, NULL, NULL, NULL, NULL, &sd);
            put("named missing ");
            number((long)e);
            nl();
        }
    }
    /* Absolute descriptors. */
    {
        SECURITY_DESCRIPTOR abs;
        PACL acl = (PACL)b;
        BYTE everyone[SECURITY_MAX_SID_SIZE];
        DWORD es = sizeof everyone, rel = 0;
        BOOL present = 0, defaulted = 0;
        PACL got = NULL;
        CreateWellKnownSid(WinWorldSid, NULL, everyone, &es);
        result("init-sd", InitializeSecurityDescriptor(&abs, SECURITY_DESCRIPTOR_REVISION));
        put("init-sd control=");
        hex(abs.Control);
        put(" valid=");
        number(IsValidSecurityDescriptor(&abs));
        put(" length=");
        number((long)GetSecurityDescriptorLength(&abs));
        nl();
        result("init-acl", InitializeAcl(acl, 28, ACL_REVISION));
        result("add-ace", AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_READ, everyone));
        result("add-ace-full", AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_READ, everyone));
        result("init-acl-small", InitializeAcl((PACL)(b + 64), 4, ACL_REVISION));
        result("set-dacl", SetSecurityDescriptorDacl(&abs, TRUE, acl, FALSE));
        result("get-dacl", GetSecurityDescriptorDacl(&abs, &present, &got, &defaulted));
        put("get-dacl present=");
        number(present);
        put(" same=");
        number(got == acl);
        put(" length=");
        number((long)GetSecurityDescriptorLength(&abs));
        nl();
        put("make-relative ");
        if (MakeSelfRelativeSD(&abs, b + 256, &rel)) put("ok");
        else {
            put("error ");
            number((long)GetLastError());
        }
        put(" needed=");
        number((long)rel);
        nl();
        result("make-relative2", MakeSelfRelativeSD(&abs, b + 256, &rel));
        put("relative control=");
        hex(((SECURITY_DESCRIPTOR_RELATIVE *)(b + 256))->Control);
        nl();
        result("make-relative-again", MakeSelfRelativeSD(b + 256, b + 512, &rel));
        result("set-dacl-relative", SetSecurityDescriptorDacl(b + 256, TRUE, acl, FALSE));
        {
            ACL_SIZE_INFORMATION size;
            GetAclInformation(acl, &size, sizeof size, AclSizeInformation);
            put("acl-info count=");
            number((long)size.AceCount);
            put(" used=");
            number((long)size.AclBytesInUse);
            put(" free=");
            number((long)size.AclBytesFree);
            nl();
        }
    }
    /* CryptoAPI. */
    {
        HCRYPTPROV aes = 0, full = 0;
        BYTE random[32];
        DWORD i, zero = 0;
        result("crypt-aes", CryptAcquireContextW(&aes, NULL, NULL, PROV_RSA_AES, CRYPT_VERIFYCONTEXT));
        result("crypt-full", CryptAcquireContextW(&full, NULL, NULL, PROV_RSA_FULL, CRYPT_VERIFYCONTEXT));
        hash_dump(aes, CALG_MD5, "aes-md5");
        hash_dump(aes, CALG_SHA1, "aes-sha1");
        hash_dump(aes, CALG_SHA_256, "aes-sha256");
        hash_dump(aes, CALG_SHA_384, "aes-sha384");
        hash_dump(aes, CALG_SHA_512, "aes-sha512");
        hash_dump(full, CALG_SHA_256, "full-sha256");
        hash_dump(full, CALG_MD4, "full-md4");
        hash_dump(full, CALG_MD2, "full-md2");
        hash_dump(full, CALG_HMAC, "full-hmac");
        result("random", CryptGenRandom(aes, sizeof random, random));
        for (i = 0; i < sizeof random; i++) zero += random[i] == 0;
        put("random zeros=");
        number((long)zero);
        nl();
        put("provider-name ");
        {
            char name[128];
            DWORD nl2 = sizeof name, type = 0, tl = sizeof type;
            if (CryptGetProvParam(aes, PP_NAME, (BYTE *)name, &nl2, 0)) put(name);
            CryptGetProvParam(aes, PP_PROVTYPE, (BYTE *)&type, &tl, 0);
            put(" type=");
            number((long)type);
        }
        nl();
        result("crypt-release", CryptReleaseContext(aes, 0));
        result("crypt-release-twice", CryptReleaseContext(aes, 0));
        CryptReleaseContext(full, 0);
        {
            HCRYPTPROV bad = 0;
            result("crypt-bad-type", CryptAcquireContextW(&bad, NULL, NULL, 999, CRYPT_VERIFYCONTEXT));
        }
    }
    CloseHandle(token);
}

static int arg_is(const wchar_t *line, const wchar_t *word) {
    /* The first argument after the program name. */
    int quoted = 0;
    while (*line && (quoted || (*line != L' ' && *line != L'\t'))) {
        if (*line == L'"') quoted = !quoted;
        line++;
    }
    while (*line == L' ' || *line == L'\t') line++;
    while (*word && *line == *word) line++, word++;
    return *word == 0 && (*line == 0 || *line == L' ');
}

/* ---- SECURITY-A checks --------------------------------------------------------------- */
enum { STANDARD = 1, FULL = 2, LIMITED = 3 };
static int context;  /* the token type the run expects (TokenElevationType) */
static int guest;    /* key containers too (they write the profile) */
static const char *tag = "SEC-A";
static int failed_step;
static DWORD fail_error;
static void step(int id, const char *name, int ok, long detail) {
    put(tag);
    put(" ");
    number(id);
    put(" ");
    put(name);
    put(ok ? " ok " : " FAILED ");
    number(detail);
    if (!ok) {
        put(" error=");
        hex(fail_error);
    }
    put("\r\n");
    if (!ok && !failed_step) failed_step = id;
}
/* The first failing check of a step, for its detail, and its last error. */
static int check;
#define EXPECT(cond)                     \
    do {                                 \
        check++;                         \
        if (!(cond)) {                   \
            fail_error = GetLastError(); \
            return check;                \
        }                                \
    } while (0)
static int wsame(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static BYTE user_sid[SECURITY_MAX_SID_SIZE];
static BYTE owner_sid[SECURITY_MAX_SID_SIZE];
static HANDLE process_token;
static BYTE scratch[8192];

static int token_step(void) {
    DWORD n = 0, v = 0;
    TOKEN_GROUPS *g = (TOKEN_GROUPS *)scratch;
    BYTE ba[SECURITY_MAX_SID_SIZE];
    DWORD bs = sizeof ba;
    BOOL member = 2;
    HANDLE linked = 0;
    check = 0;
    EXPECT(OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &process_token));
    EXPECT(GetTokenInformation(process_token, TokenElevationType, &v, 4, &n) && (int)v == context);
    EXPECT(GetTokenInformation(process_token, TokenElevation, &v, 4, &n) && v == (DWORD)(context == FULL));
    EXPECT(GetTokenInformation(process_token, TokenUser, scratch, sizeof scratch, &n));
    EXPECT(CopySid(sizeof user_sid, user_sid, ((TOKEN_USER *)scratch)->User.Sid));
    /* A local account: S-1-5-21, the machine's three parts, the RID. */
    EXPECT(*GetSidSubAuthorityCount(user_sid) == 5 && *GetSidSubAuthority(user_sid, 0) == 21);
    EXPECT(GetTokenInformation(process_token, TokenOwner, scratch, sizeof scratch, &n));
    EXPECT(CopySid(sizeof owner_sid, owner_sid, ((TOKEN_OWNER *)scratch)->Owner));
    CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, ba, &bs);
    EXPECT(EqualSid(owner_sid, context == FULL ? (PSID)ba : (PSID)user_sid));
    /* The integrity label is the first group. */
    EXPECT(GetTokenInformation(process_token, TokenGroups, scratch, sizeof scratch, &n));
    EXPECT(*GetSidSubAuthority(g->Groups[0].Sid, 0) == (context == FULL ? 0x3000u : 0x2000u));
    {
        DWORD i, admins = 0;
        for (i = 0; i < g->GroupCount; i++)
            if (EqualSid(g->Groups[i].Sid, ba)) admins = g->Groups[i].Attributes;
        if (context == STANDARD) EXPECT(admins == 0);
        else if (context == LIMITED) EXPECT(admins == SE_GROUP_USE_FOR_DENY_ONLY);
        else EXPECT(admins & SE_GROUP_ENABLED);
    }
    /* The linked token: exactly one handle's buffer. */
    EXPECT(!GetTokenInformation(process_token, TokenLinkedToken, scratch, 16, &n) &&
           GetLastError() == ERROR_BAD_LENGTH && n == 8);
    if (context == STANDARD) {
        EXPECT(!GetTokenInformation(process_token, TokenLinkedToken, &linked, 8, &n));
    } else {
        EXPECT(GetTokenInformation(process_token, TokenLinkedToken, &linked, 8, &n));
        EXPECT(GetTokenInformation(linked, TokenType, &v, 4, &n) && v == TokenImpersonation);
        EXPECT(GetTokenInformation(linked, TokenImpersonationLevel, &v, 4, &n) &&
               v == SecurityIdentification);
        EXPECT(GetTokenInformation(linked, TokenElevationType, &v, 4, &n) &&
               v == (DWORD)(context == FULL ? LIMITED : FULL));
        CloseHandle(linked);
    }
    EXPECT(CheckTokenMembership(NULL, ba, &member) && member == (context == FULL));
    EXPECT(!CheckTokenMembership(process_token, ba, &member) &&
           GetLastError() == ERROR_NO_IMPERSONATION_TOKEN);
    EXPECT(!GetTokenInformation(process_token, TokenImpersonationLevel, &v, 4, &n) &&
           GetLastError() == ERROR_INVALID_PARAMETER);
    EXPECT(GetTokenInformation(process_token, TokenPrivileges, scratch, sizeof scratch, &n));
    EXPECT(context == FULL ? ((TOKEN_PRIVILEGES *)scratch)->PrivilegeCount > 5
                           : ((TOKEN_PRIVILEGES *)scratch)->PrivilegeCount == 5);
    return 0;
}

static int sids_step(void) {
    BYTE sid[SECURITY_MAX_SID_SIZE];
    DWORD size = 4, dl, nl;
    wchar_t name[128], domain[128];
    SID_NAME_USE use;
    PSID converted = NULL;
    LPWSTR text = NULL;
    check = 0;
    EXPECT(!CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, sid, &size) &&
           GetLastError() == ERROR_INSUFFICIENT_BUFFER && size == 16);
    EXPECT(CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, sid, &size));
    EXPECT(ConvertSidToStringSidW(sid, &text) && wsame(text, L"S-1-5-32-544"));
    LocalFree(text);
    size = sizeof sid;
    EXPECT(!CreateWellKnownSid(WinLogonIdsSid, NULL, sid, &size) &&
           GetLastError() == ERROR_INVALID_PARAMETER);
    size = sizeof sid;
    EXPECT(CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, sid, &size));
    EXPECT(IsWellKnownSid(sid, WinBuiltinAdministratorsSid) && !IsWellKnownSid(sid, WinBuiltinUsersSid));
    EXPECT(ConvertStringSidToSidW(L"BA", &converted) && EqualSid(converted, sid));
    LocalFree(converted);
    nl = dl = 128;
    EXPECT(LookupAccountSidW(NULL, sid, name, &nl, domain, &dl, &use) &&
           wsame(name, L"Administrators") && wsame(domain, L"BUILTIN") && use == SidTypeAlias);
    size = sizeof sid;
    dl = 128;
    EXPECT(LookupAccountNameW(NULL, L"NT AUTHORITY\\SYSTEM", sid, &size, domain, &dl, &use) &&
           IsWellKnownSid(sid, WinLocalSystemSid) && wsame(domain, L"NT AUTHORITY") &&
           use == SidTypeWellKnownGroup);
    nl = 128;
    EXPECT(GetUserNameW(name, &nl));
    size = sizeof sid;
    dl = 128;
    EXPECT(LookupAccountNameW(NULL, name, sid, &size, domain, &dl, &use) && EqualSid(sid, user_sid) &&
           use == SidTypeUser);
    size = sizeof sid;
    dl = 128;
    EXPECT(!LookupAccountNameW(NULL, L"NoSuchAccountDoors", sid, &size, domain, &dl, &use) &&
           GetLastError() == ERROR_NONE_MAPPED);
    size = sizeof sid;
    EXPECT(GetWindowsAccountDomainSid(user_sid, sid, &size) && *GetSidSubAuthorityCount(sid) == 4);
    {
        DWORD i;
        for (i = 0; i < 4; i++) EXPECT(*GetSidSubAuthority(sid, i) == *GetSidSubAuthority(user_sid, i));
    }
    return 0;
}

static int descriptors_step(void) {
    SECURITY_DESCRIPTOR abs;
    PACL acl = (PACL)scratch;
    BYTE everyone[SECURITY_MAX_SID_SIZE];
    DWORD es = sizeof everyone, rel = 0;
    BOOL present = 0, defaulted = 0;
    PACL got = NULL;
    ACL_SIZE_INFORMATION size;
    void *ace = NULL;
    check = 0;
    CreateWellKnownSid(WinWorldSid, NULL, everyone, &es);
    EXPECT(InitializeSecurityDescriptor(&abs, SECURITY_DESCRIPTOR_REVISION));
    EXPECT(abs.Control == 0 && IsValidSecurityDescriptor(&abs) && GetSecurityDescriptorLength(&abs) == 40);
    EXPECT(!InitializeAcl((PACL)(scratch + 64), 4, ACL_REVISION) && GetLastError() == ERROR_INSUFFICIENT_BUFFER);
    EXPECT(InitializeAcl(acl, 28, ACL_REVISION));
    EXPECT(AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_READ, everyone));
    EXPECT(!AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_READ, everyone) &&
           GetLastError() == ERROR_ALLOTTED_SPACE_EXCEEDED);
    EXPECT(GetAclInformation(acl, &size, sizeof size, AclSizeInformation) && size.AceCount == 1 &&
           size.AclBytesInUse == 28 && size.AclBytesFree == 0);
    EXPECT(GetAce(acl, 0, &ace) && ((ACCESS_ALLOWED_ACE *)ace)->Mask == GENERIC_READ);
    EXPECT(SetSecurityDescriptorDacl(&abs, TRUE, acl, FALSE));
    EXPECT(GetSecurityDescriptorDacl(&abs, &present, &got, &defaulted) && present && got == acl);
    EXPECT(GetSecurityDescriptorLength(&abs) == 68);
    EXPECT(!MakeSelfRelativeSD(&abs, scratch + 256, &rel) && GetLastError() == ERROR_INSUFFICIENT_BUFFER &&
           rel == 48);
    EXPECT(MakeSelfRelativeSD(&abs, scratch + 256, &rel));
    EXPECT(((SECURITY_DESCRIPTOR_RELATIVE *)(scratch + 256))->Control == 0x8004);
    EXPECT(GetSecurityDescriptorLength(scratch + 256) == 48);
    EXPECT(!MakeSelfRelativeSD(scratch + 256, scratch + 512, &rel) && GetLastError() == ERROR_BAD_DESCRIPTOR_FORMAT);
    EXPECT(!SetSecurityDescriptorDacl(scratch + 256, TRUE, acl, FALSE) &&
           GetLastError() == ERROR_INVALID_SECURITY_DESCR);
    EXPECT(DeleteAce(acl, 0) && GetAclInformation(acl, &size, sizeof size, AclSizeInformation) &&
           size.AceCount == 0 && size.AclBytesInUse == 8);
    return 0;
}

static const struct {
    const wchar_t *text;
    ULONG size;
    /* The length ConvertSecurityDescriptorToStringSecurityDescriptor reports. */
    ULONG length;
} samples[] = {
    {L"O:BAG:SYD:P(A;OICI;FA;;;SY)(A;;0x1200a9;;;BU)S:(ML;;NW;;;ME)", 128, 65},
    {L"O:SYG:SYD:AI(A;ID;FA;;;SY)(A;OICIIOID;GA;;;BA)(D;;WD;;;WD)", 116, 61},
    {L"D:(A;;GA;;;WD)", 48, 15},
    {L"O:BA", 36, 4},
    {L"D:NO_ACCESS_CONTROL", 20, 20},
    {L"D:", 28, 2},
    {L"G:S-1-5-21-1-2-3-513D:(A;;KR;;;AU)(A;CI;KA;;;CO)", 96, 50},
    {L"S:(ML;OICINPIO;NWNRNX;;;HI)", 48, 28},
};
static int sddl_step(void) {
    static const struct {
        const wchar_t *text;
        DWORD error;
    } bad[] = {
        {L"O:XX", 1337}, {L"D:(Q;;GA;;;WD)", 1804}, {L"D:(A;;GA;;;WD", 1336},
        {L"garbage", 87}, {L"D:(A;;ZZ;;;WD)", 1336}, {L"D:(A;XX;GA;;;WD)", 1004},
    };
    DWORD i;
    check = 0;
    for (i = 0; i < sizeof samples / sizeof samples[0]; i++) {
        PSECURITY_DESCRIPTOR sd = NULL;
        ULONG size = 0, length = 0;
        LPWSTR back = NULL;
        EXPECT(ConvertStringSecurityDescriptorToSecurityDescriptorW(samples[i].text, SDDL_REVISION_1, &sd, &size));
        EXPECT(size == samples[i].size && GetSecurityDescriptorLength(sd) == size);
        EXPECT(ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd, SDDL_REVISION_1,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION |
                LABEL_SECURITY_INFORMATION,
            &back, &length));
        EXPECT(wsame(back, samples[i].text));
        EXPECT(length == samples[i].length);
        LocalFree(back);
        LocalFree(sd);
    }
    for (i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        PSECURITY_DESCRIPTOR sd = NULL;
        EXPECT(!ConvertStringSecurityDescriptorToSecurityDescriptorW(bad[i].text, SDDL_REVISION_1, &sd, NULL) &&
               GetLastError() == bad[i].error);
    }
    return 0;
}

static DWORD checked(HANDLE token, const wchar_t *sddl, DWORD want, BOOL *status) {
    PSECURITY_DESCRIPTOR sd = NULL;
    GENERIC_MAPPING map = {FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_GENERIC_EXECUTE, FILE_ALL_ACCESS};
    PRIVILEGE_SET privileges;
    DWORD plen = sizeof privileges, granted = 0xdead;
    *status = 2;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &sd, NULL);
    if (!AccessCheck(sd, token, want, &map, &privileges, &plen, &granted, status)) {
        granted = 0xE0000000 | GetLastError();
    }
    LocalFree(sd);
    return granted;
}
static int access_step(void) {
    HANDLE thread = 0, dup = 0;
    DWORD v = 0, n;
    BOOL status;
    check = 0;
    EXPECT(!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread) && GetLastError() == ERROR_NO_TOKEN);
    EXPECT(ImpersonateSelf(SecurityImpersonation));
    EXPECT(OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread));
    EXPECT(GetTokenInformation(thread, TokenType, &v, 4, &n) && v == TokenImpersonation);
    EXPECT(GetTokenInformation(thread, TokenImpersonationLevel, &v, 4, &n) && v == SecurityImpersonation);
    EXPECT(checked(thread, L"O:SYG:SYD:(A;;FR;;;BU)", MAXIMUM_ALLOWED, &status) == 0x120089 && status);
    EXPECT(checked(thread, L"O:SYG:SYD:(A;;FR;;;BU)", FILE_READ_DATA, &status) == 1 && status);
    /* The denial reaches the Administrators group (for deny only when
     * filtered); a standard user has none. */
    EXPECT(checked(thread, L"O:SYG:SYD:(D;;0x2;;;BA)(A;;FA;;;BU)", MAXIMUM_ALLOWED, &status) ==
           (context == STANDARD ? 0x1f01ffu : 0x1f01fdu));
    EXPECT(checked(thread, L"O:SYG:SYD:NO_ACCESS_CONTROL", MAXIMUM_ALLOWED, &status) == 0x1f01ff);
    EXPECT(checked(thread, L"O:SYG:SYD:", FILE_READ_DATA, &status) == 0 && !status &&
           GetLastError() == ERROR_ACCESS_DENIED);
    EXPECT(checked(thread, L"D:(A;;FA;;;WD)", FILE_READ_DATA, &status) == (0xE0000000 | ERROR_INVALID_SECURITY_DESCR));
    EXPECT(checked(thread, L"O:SYG:SYD:(A;;GA;;;WD)", MAXIMUM_ALLOWED, &status) == GENERIC_ALL);
    EXPECT(checked(thread, L"O:SYG:SYD:(A;;FR;;;BU)", GENERIC_READ, &status) == (0xE0000000 | ERROR_GENERIC_NOT_MAPPED));
    EXPECT(checked(process_token, L"O:SYG:SYD:(A;;FR;;;BU)", FILE_READ_DATA, &status) ==
           (0xE0000000 | ERROR_NO_IMPERSONATION_TOKEN));
    CloseHandle(thread);
    EXPECT(RevertToSelf());
    EXPECT(!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread) && GetLastError() == ERROR_NO_TOKEN);
    EXPECT(DuplicateToken(process_token, SecurityIdentification, &dup));
    EXPECT(GetTokenInformation(dup, TokenType, &v, 4, &n) && v == TokenImpersonation);
    EXPECT(GetTokenInformation(dup, TokenImpersonationLevel, &v, 4, &n) && v == SecurityIdentification);
    CloseHandle(dup);
    return 0;
}

static DWORD privilege_state(DWORD luid) {
    DWORD n, i;
    TOKEN_PRIVILEGES *p = (TOKEN_PRIVILEGES *)scratch;
    if (!GetTokenInformation(process_token, TokenPrivileges, scratch, sizeof scratch, &n)) return 0xdead;
    for (i = 0; i < p->PrivilegeCount; i++)
        if (p->Privileges[i].Luid.LowPart == luid) return p->Privileges[i].Attributes;
    return 0xffff;
}
static int privileges_step(void) {
    TOKEN_PRIVILEGES tp, previous;
    DWORD plen = sizeof previous;
    check = 0;
    tp.PrivilegeCount = 1;
    tp.Privileges[0].Luid.LowPart = 19; /* SeShutdownPrivilege */
    tp.Privileges[0].Luid.HighPart = 0;
    tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    EXPECT(privilege_state(19) == 0);
    EXPECT(AdjustTokenPrivileges(process_token, FALSE, &tp, sizeof previous, &previous, &plen) &&
           GetLastError() == 0);
    EXPECT(previous.PrivilegeCount == 1 && previous.Privileges[0].Luid.LowPart == 19 &&
           previous.Privileges[0].Attributes == 0);
    EXPECT(privilege_state(19) == SE_PRIVILEGE_ENABLED);
    tp.Privileges[0].Attributes = 0;
    EXPECT(AdjustTokenPrivileges(process_token, FALSE, &tp, 0, NULL, NULL));
    EXPECT(privilege_state(19) == 0);
    tp.Privileges[0].Luid.LowPart = 7; /* SeTcbPrivilege: held by nobody here */
    tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    EXPECT(AdjustTokenPrivileges(process_token, FALSE, &tp, 0, NULL, NULL) &&
           GetLastError() == ERROR_NOT_ALL_ASSIGNED);
    return 0;
}

/* The DACL has an allowed ACE for `sid` with exactly `mask`. */
static int grants(PACL acl, PSID sid, DWORD mask) {
    DWORD i;
    for (i = 0; acl && i < acl->AceCount; i++) {
        ACCESS_ALLOWED_ACE *a;
        if (GetAce(acl, i, (void **)&a) && a->Header.AceType == ACCESS_ALLOWED_ACE_TYPE &&
            EqualSid(&a->SidStart, sid) && a->Mask == mask)
            return 1;
    }
    return 0;
}
static int file_step(void) {
    wchar_t path[MAX_PATH];
    DWORD n = GetTempPathW(MAX_PATH - 20, path), needed = 0;
    PSID owner = NULL;
    PACL dacl = NULL, fresh = NULL;
    PSECURITY_DESCRIPTOR sd = NULL;
    BYTE everyone[SECURITY_MAX_SID_SIZE];
    DWORD es = sizeof everyone;
    HANDLE file, thread = 0;
    BOOL status;
    const wchar_t *leaf = L"secprobe.tmp";
    int i = 0;
    check = 0;
    EXPECT(n > 0);
    while (leaf[i]) path[n++] = leaf[i++];
    path[n] = 0;
    CreateWellKnownSid(WinWorldSid, NULL, everyone, &es);
    file = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    EXPECT(file != INVALID_HANDLE_VALUE);
    CloseHandle(file);
    EXPECT(GetNamedSecurityInfoW(path, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                                 &owner, NULL, &dacl, NULL, &sd) == ERROR_SUCCESS);
    EXPECT(EqualSid(owner, user_sid) || EqualSid(owner, owner_sid));
    EXPECT(grants(dacl, owner, FILE_ALL_ACCESS) || grants(dacl, user_sid, FILE_ALL_ACCESS));
    LocalFree(sd);
    /* Share it: the user full control, Everyone read; protected. */
    {
        PACL acl = (PACL)scratch;
        InitializeAcl(acl, 128, ACL_REVISION);
        AddAccessAllowedAce(acl, ACL_REVISION, FILE_ALL_ACCESS, user_sid);
        AddAccessAllowedAce(acl, ACL_REVISION, FILE_GENERIC_READ, everyone);
        EXPECT(SetNamedSecurityInfoW(path, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                                     NULL, NULL, acl, NULL) == ERROR_SUCCESS);
    }
    EXPECT(GetNamedSecurityInfoW(path, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, NULL, NULL, &dacl, NULL, &sd) ==
           ERROR_SUCCESS);
    EXPECT(grants(dacl, everyone, FILE_GENERIC_READ) && grants(dacl, user_sid, FILE_ALL_ACCESS));
    LocalFree(sd);
    EXPECT(!GetFileSecurityW(path, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, scratch, 8, &needed) &&
           GetLastError() == ERROR_INSUFFICIENT_BUFFER && needed > 20);
    EXPECT(GetFileSecurityW(path, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, scratch, sizeof scratch,
                            &needed) &&
           IsValidSecurityDescriptor(scratch) &&
           (((SECURITY_DESCRIPTOR_RELATIVE *)scratch)->Control & SE_DACL_PRESENT));
    /* The open file: its owner, then Everyone removed through the handle. */
    file = CreateFileW(path, GENERIC_READ | READ_CONTROL | WRITE_DAC, FILE_SHARE_READ, NULL, OPEN_EXISTING,
                       FILE_ATTRIBUTE_NORMAL, NULL);
    EXPECT(file != INVALID_HANDLE_VALUE);
    EXPECT(GetSecurityInfo(file, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &owner, NULL, NULL, NULL, &sd) ==
           ERROR_SUCCESS);
    EXPECT(EqualSid(owner, user_sid) || EqualSid(owner, owner_sid));
    LocalFree(sd);
    fresh = (PACL)(scratch + 512);
    InitializeAcl(fresh, 64, ACL_REVISION);
    AddAccessAllowedAce(fresh, ACL_REVISION, FILE_ALL_ACCESS, user_sid);
    EXPECT(SetSecurityInfo(file, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                           NULL, NULL, fresh, NULL) == ERROR_SUCCESS);
    CloseHandle(file);
    EXPECT(GetNamedSecurityInfoW(path, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, NULL, NULL, &dacl, NULL, &sd) ==
           ERROR_SUCCESS);
    EXPECT(!grants(dacl, everyone, FILE_GENERIC_READ) && grants(dacl, user_sid, FILE_ALL_ACCESS));
    LocalFree(sd);
    EXPECT(DeleteFileW(path));
    /* C:\Program Files: adding a file needs the full token. */
    EXPECT(GetNamedSecurityInfoW(L"C:\\Program Files", SE_FILE_OBJECT,
                                 OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                                 NULL, NULL, NULL, NULL, &sd) == ERROR_SUCCESS);
    EXPECT(ImpersonateSelf(SecurityImpersonation) && OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread));
    {
        GENERIC_MAPPING map = {FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_GENERIC_EXECUTE, FILE_ALL_ACCESS};
        PRIVILEGE_SET privileges;
        DWORD plen = sizeof privileges, granted = 0;
        EXPECT(AccessCheck(sd, thread, FILE_ADD_FILE, &map, &privileges, &plen, &granted, &status));
        EXPECT(status == (context == FULL));
        EXPECT(AccessCheck(sd, thread, FILE_LIST_DIRECTORY, &map, &privileges, &plen, &granted, &status) && status);
    }
    CloseHandle(thread);
    RevertToSelf();
    LocalFree(sd);
    EXPECT(GetNamedSecurityInfoW(L"C:\\no\\such\\file.txt", SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, NULL, NULL,
                                 NULL, NULL, &sd) == ERROR_PATH_NOT_FOUND);
    return 0;
}

static int registry_step(void) {
    PSID owner = NULL;
    PACL dacl = NULL;
    PSECURITY_DESCRIPTOR sd = NULL;
    BYTE users[SECURITY_MAX_SID_SIZE], ba[SECURITY_MAX_SID_SIZE];
    DWORD us = sizeof users, bs = sizeof ba, length = sizeof scratch;
    HKEY key;
    LONG e;
    BOOL present, defaulted;
    check = 0;
    CreateWellKnownSid(WinBuiltinUsersSid, NULL, users, &us);
    CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, ba, &bs);
    EXPECT(GetNamedSecurityInfoW(L"MACHINE\\SOFTWARE", SE_REGISTRY_KEY,
                                 OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &owner, NULL, &dacl, NULL,
                                 &sd) == ERROR_SUCCESS);
    EXPECT(EqualSid(owner, ba) && grants(dacl, users, KEY_READ) && grants(dacl, ba, KEY_ALL_ACCESS));
    LocalFree(sd);
    e = RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE", 0, KEY_SET_VALUE, &key);
    if (context == FULL) {
        EXPECT(e == ERROR_SUCCESS);
        RegCloseKey(key);
    } else {
        EXPECT(e == ERROR_ACCESS_DENIED);
    }
    EXPECT(RegOpenKeyExW(HKEY_CURRENT_USER, L"Software", 0, KEY_READ, &key) == ERROR_SUCCESS);
    EXPECT(RegGetKeySecurity(key, DACL_SECURITY_INFORMATION, scratch, &length) == ERROR_SUCCESS);
    EXPECT(GetSecurityDescriptorDacl(scratch, &present, &dacl, &defaulted) && present &&
           grants(dacl, user_sid, KEY_ALL_ACCESS));
    RegCloseKey(key);
    return 0;
}

static int kernel_step(void) {
    PSID owner = NULL;
    PACL dacl = NULL;
    PSECURITY_DESCRIPTOR sd = NULL;
    check = 0;
    EXPECT(GetSecurityInfo(GetCurrentProcess(), SE_KERNEL_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                           &owner, NULL, &dacl, NULL, &sd) == ERROR_SUCCESS);
    EXPECT(EqualSid(owner, owner_sid) && grants(dacl, owner_sid, PROCESS_ALL_ACCESS));
    LocalFree(sd);
    return 0;
}

static int entries_step(void) {
    EXPLICIT_ACCESS_W e[2];
    PACL acl = NULL, next = NULL;
    BYTE users[SECURITY_MAX_SID_SIZE], everyone[SECURITY_MAX_SID_SIZE];
    DWORD us = sizeof users, es = sizeof everyone;
    ACCESS_ALLOWED_ACE *a;
    check = 0;
    CreateWellKnownSid(WinBuiltinUsersSid, NULL, users, &us);
    CreateWellKnownSid(WinWorldSid, NULL, everyone, &es);
    memset(e, 0, sizeof e);
    e[0].grfAccessPermissions = FILE_GENERIC_READ;
    e[0].grfAccessMode = GRANT_ACCESS;
    BuildTrusteeWithSidW(&e[0].Trustee, users);
    e[1].grfAccessPermissions = WRITE_DAC;
    e[1].grfAccessMode = DENY_ACCESS;
    BuildTrusteeWithNameW(&e[1].Trustee, L"Everyone");
    EXPECT(SetEntriesInAclW(2, e, NULL, &acl) == ERROR_SUCCESS && acl->AceCount == 2);
    EXPECT(GetAce(acl, 0, (void **)&a) && a->Header.AceType == ACCESS_DENIED_ACE_TYPE &&
           EqualSid(&a->SidStart, everyone) && a->Mask == WRITE_DAC);
    EXPECT(grants(acl, users, FILE_GENERIC_READ));
    e[0].grfAccessPermissions = FILE_ALL_ACCESS;
    e[0].grfAccessMode = SET_ACCESS;
    EXPECT(SetEntriesInAclW(1, e, acl, &next) == ERROR_SUCCESS && next->AceCount == 2 &&
           grants(next, users, FILE_ALL_ACCESS));
    LocalFree(acl);
    /* REVOKE_ACCESS removes the trustee's grants, not its denials. */
    e[1].grfAccessMode = REVOKE_ACCESS;
    EXPECT(SetEntriesInAclW(1, &e[1], next, &acl) == ERROR_SUCCESS && acl->AceCount == 2);
    LocalFree(acl);
    e[0].grfAccessMode = REVOKE_ACCESS;
    EXPECT(SetEntriesInAclW(1, e, next, &acl) == ERROR_SUCCESS && acl->AceCount == 1 &&
           !grants(acl, users, FILE_ALL_ACCESS));
    LocalFree(next);
    LocalFree(acl);
    return 0;
}

static int digest_is(HCRYPTPROV prov, ALG_ID alg, const char *hexdigest) {
    HCRYPTHASH h = 0;
    BYTE d[64];
    DWORD n = sizeof d, k;
    if (!CryptCreateHash(prov, alg, 0, 0, &h)) return 0;
    if (!CryptHashData(h, (const BYTE *)"a", 1, 0) || !CryptHashData(h, (const BYTE *)"bc", 2, 0)) return 0;
    if (!CryptGetHashParam(h, HP_HASHVAL, d, &n, 0)) return 0;
    for (k = 0; k < n; k++) {
        if (hexdigest[2 * k] != "0123456789abcdef"[d[k] >> 4] ||
            hexdigest[2 * k + 1] != "0123456789abcdef"[d[k] & 15])
            return 0;
    }
    if (hexdigest[2 * n]) return 0;
    if (CryptHashData(h, (const BYTE *)"x", 1, 0) || GetLastError() != (DWORD)NTE_BAD_HASH_STATE) return 0;
    CryptDestroyHash(h);
    return 1;
}
static int crypto_step(void) {
    HCRYPTPROV aes = 0, full = 0, box = 0;
    HCRYPTHASH h = 0;
    HCRYPTKEY key = 0;
    BYTE random[32];
    char name[128];
    DWORD i, zero = 0, nl = sizeof name;
    check = 0;
    EXPECT(CryptAcquireContextW(&aes, NULL, NULL, PROV_RSA_AES, CRYPT_VERIFYCONTEXT));
    EXPECT(CryptAcquireContextW(&full, NULL, NULL, PROV_RSA_FULL, CRYPT_VERIFYCONTEXT));
    EXPECT(digest_is(aes, CALG_MD5, "900150983cd24fb0d6963f7d28e17f72"));
    EXPECT(digest_is(aes, CALG_SHA1, "a9993e364706816aba3e25717850c26c9cd0d89d"));
    EXPECT(digest_is(aes, CALG_SHA_256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    EXPECT(digest_is(aes, CALG_SHA_512, "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a"
                                        "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"));
    EXPECT(digest_is(full, CALG_MD4, "a448017aaf21d8525fc10ae87aa6729d"));
    EXPECT(!CryptCreateHash(full, CALG_SHA_256, 0, 0, &h) && GetLastError() == (DWORD)NTE_BAD_ALGID);
    EXPECT(!CryptCreateHash(full, CALG_HMAC, 0, 0, &h) && GetLastError() == (DWORD)NTE_BAD_KEY);
    EXPECT(CryptGenRandom(aes, sizeof random, random));
    for (i = 0; i < sizeof random; i++) zero += random[i] == 0;
    EXPECT(zero < 8);
    EXPECT(CryptGetProvParam(aes, PP_NAME, (BYTE *)name, &nl, 0));
    EXPECT(nl == sizeof MS_ENH_RSA_AES_PROV_A && name[10] == 'E');
    EXPECT(CryptReleaseContext(aes, 0));
    EXPECT(!CryptReleaseContext(aes, 0) && GetLastError() == ERROR_INVALID_PARAMETER);
    CryptReleaseContext(full, 0);
    EXPECT(!CryptAcquireContextW(&box, NULL, NULL, 999, CRYPT_VERIFYCONTEXT) &&
           GetLastError() == (DWORD)NTE_PROV_TYPE_NOT_DEF);
    EXPECT(!CryptAcquireContextW(&box, NULL, MS_DEF_PROV_W, PROV_RSA_AES, CRYPT_VERIFYCONTEXT) &&
           GetLastError() == (DWORD)NTE_PROV_TYPE_NO_MATCH);
    EXPECT(!CryptAcquireContextW(&box, NULL, L"No Such Provider", PROV_RSA_FULL, CRYPT_VERIFYCONTEXT) &&
           GetLastError() == (DWORD)NTE_KEYSET_NOT_DEF);
    EXPECT(CryptAcquireContextW(&box, NULL, MS_ENHANCED_PROV_W, PROV_RSA_FULL, CRYPT_VERIFYCONTEXT));
    nl = sizeof name;
    EXPECT(CryptGetProvParam(box, PP_NAME, (BYTE *)name, &nl, 0) && nl == sizeof MS_ENHANCED_PROV_A);
    CryptReleaseContext(box, 0);
    if (guest) {
        /* A key container in the user's profile: absent, made, kept, gone. */
        const wchar_t *name_w = L"DoorsProbeContainer";
        EXPECT(!CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, 0) && GetLastError() == (DWORD)NTE_BAD_KEYSET);
        EXPECT(CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, CRYPT_NEWKEYSET));
        EXPECT(!CryptGetUserKey(box, AT_KEYEXCHANGE, &key) && GetLastError() == (DWORD)NTE_NO_KEY);
        CryptReleaseContext(box, 0);
        EXPECT(!CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, CRYPT_NEWKEYSET) &&
               GetLastError() == (DWORD)NTE_EXISTS);
        EXPECT(CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, 0));
        CryptReleaseContext(box, 0);
        EXPECT(CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, CRYPT_DELETEKEYSET));
        EXPECT(!CryptAcquireContextW(&box, name_w, NULL, PROV_RSA_FULL, 0) && GetLastError() == (DWORD)NTE_BAD_KEYSET);
    }
    return 0;
}

static void checks(void) {
    static const struct {
        const char *name;
        int (*run)(void);
    } steps[] = {
        {"token", token_step},
        {"sids", sids_step},
        {"descriptors", descriptors_step},
        {"sddl", sddl_step},
        {"access-check", access_step},
        {"privileges", privileges_step},
        {"file-security", file_step},
        {"registry-security", registry_step},
        {"kernel-object", kernel_step},
        {"explicit-entries", entries_step},
        {"crypto", crypto_step},
    };
    DWORD i;
    for (i = 0; i < sizeof steps / sizeof steps[0]; i++) {
        int failed = steps[i].run();
        step((int)i + 1, steps[i].name, failed == 0, failed ? failed : 0);
    }
    put(tag);
    put(failed_step ? " FAILED\r\n" : " PASSED\r\n");
    ExitProcess(failed_step ? 1 : 0);
}

void entry(void) {
    const wchar_t *line = GetCommandLineW();
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    if (arg_is(line, L"dump")) {
        dump();
        ExitProcess(0);
    }
    if (arg_is(line, L"more")) {
        more();
        ExitProcess(0);
    }
    /* secprobe limited|full|standard [guest] */
    if (arg_is(line, L"limited")) context = LIMITED, tag = "SEC-L";
    if (arg_is(line, L"full")) context = FULL, tag = "SEC-F";
    if (arg_is(line, L"standard")) context = STANDARD, tag = "SEC-S";
    if (!context) {
        put("usage: secprobe dump | more | limited|full|standard [guest]\r\n");
        ExitProcess(2);
    }
    {
        int n = wlen(line);
        guest = n > 6 && wsame(line + n - 6, L" guest");
    }
    checks();
}

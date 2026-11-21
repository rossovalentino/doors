/* TRUST-A probe (P16 M5): WinVerifyTrust and the catalog store from an
 * ordinary Windows console program (Windows SDK, no C runtime):
 *
 *   trustprobe.exe raw | check | guest
 *
 * The fixtures lie next to the program (`crates/authenticode/tests/data/
 * trust`): programs signed by a test publisher and by a revoked one, the
 * unsigned program they were made from (hello.exe), a text file and a
 * signed catalog naming hello.exe and readme.txt. The compared steps only
 * read: WinVerifyTrust with cache-only URL retrieval (no network) over the
 * fixtures, whose root no store trusts; CryptCATAdmin hashes; catalog
 * member checks. `raw` adds a System32 file's catalog on the owner's PC,
 * for the record. Their output is compared with what Windows 11 printed
 * (`trustprobe.expected.h`). `guest`, run elevated, then trusts the test
 * root (the machine Root store), checks revocation against the CRL the test
 * host serves at 10.0.2.2:28480, adds the catalog to the store, finds it by
 * hash, removes it and untrusts the root: which only the guest may do. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <wincrypt.h>
#include <wintrust.h>
#include <softpub.h>
#pragma warning(push)
#pragma warning(disable : 4201)
#include <mscat.h>
#pragma warning(pop)
#include <intrin.h>

#pragma function(memset, memcpy)
void *memset(void *d, int v, size_t n) {
    __stosb((unsigned char *)d, (unsigned char)v, n);
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    __movsb((unsigned char *)d, (const unsigned char *)s, n);
    return d;
}

static HANDLE out;
static char captured[16384];
static int captured_length, capturing;
static void put(const char *s) {
    DWORD w;
    int n = 0;
    while (s[n]) n++;
    if (capturing) {
        int k;
        for (k = 0; k < n && captured_length < (int)sizeof captured - 1; k++) captured[captured_length++] = s[k];
        captured[captured_length] = 0;
        return;
    }
    WriteFile(out, s, (DWORD)n, &w, NULL);
}
static void num(long long v) {
    char b[32];
    int i = 31, neg = v < 0;
    unsigned long long u = neg ? (unsigned long long)-v : (unsigned long long)v;
    b[i] = 0;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (neg) b[--i] = '-';
    put(b + i);
}
static void hex32(unsigned long v) {
    char b[11];
    int i;
    b[0] = '0';
    b[1] = 'x';
    for (i = 0; i < 8; i++) b[2 + i] = "0123456789abcdef"[(v >> (28 - 4 * i)) & 15];
    b[10] = 0;
    put(b);
}
static void bytes(const BYTE *p, DWORD n) {
    char b[3];
    DWORD i;
    b[2] = 0;
    for (i = 0; i < n; i++) {
        b[0] = "0123456789abcdef"[p[i] >> 4];
        b[1] = "0123456789abcdef"[p[i] & 15];
        put(b);
    }
}
static void nl(void) { put("\r\n"); }
static void wput(const wchar_t *s) {
    char b[2];
    b[1] = 0;
    for (; *s; s++) {
        b[0] = *s < 128 ? (char)*s : '?';
        put(b);
    }
}
static int wlen(const wchar_t *s) {
    int n = 0;
    while (s[n]) n++;
    return n;
}
static int wsuffix(const wchar_t *s, const wchar_t *tail) {
    int a = wlen(s), b = wlen(tail), i;
    if (a < b) return 0;
    for (i = 0; i < b; i++) {
        wchar_t x = s[a - b + i], y = tail[i];
        if (x >= L'a' && x <= L'z') x -= 32;
        if (y >= L'a' && y <= L'z') y -= 32;
        if (x != y) return 0;
    }
    return 1;
}

/* The fixtures' folder: the program's own. */
static wchar_t folder[MAX_PATH];
static int folder_length;
static wchar_t paths[8][MAX_PATH];
static int next_path;
/* A fixture's path, in one of eight buffers used in turn. */
static const wchar_t *fixture(const wchar_t *name) {
    wchar_t *p = paths[next_path++ & 7];
    int n = 0;
    while (n < folder_length) {
        p[n] = folder[n];
        n++;
    }
    while (*name && n < MAX_PATH - 1) p[n++] = *name++;
    p[n] = 0;
    return p;
}

static GUID generic = WINTRUST_ACTION_GENERIC_VERIFY_V2;
static GUID driver = DRIVER_ACTION_VERIFY;
static LONG trust(WINTRUST_DATA *d) {
    LONG r;
    d->cbStruct = sizeof *d;
    d->dwUIChoice = WTD_UI_NONE;
    d->dwStateAction = WTD_STATEACTION_VERIFY;
    r = WinVerifyTrust(NULL, &generic, d);
    d->dwStateAction = WTD_STATEACTION_CLOSE;
    WinVerifyTrust(NULL, &generic, d);
    return r;
}
static LONG trust_file(const wchar_t *path, DWORD revocation, DWORD flags) {
    WINTRUST_FILE_INFO f;
    WINTRUST_DATA d;
    memset(&f, 0, sizeof f);
    memset(&d, 0, sizeof d);
    f.cbStruct = sizeof f;
    f.pcwszFilePath = path;
    d.fdwRevocationChecks = revocation;
    d.dwUnionChoice = WTD_CHOICE_FILE;
    d.pFile = &f;
    d.dwProvFlags = flags;
    return trust(&d);
}
static DWORD file_hash(HCATADMIN admin, const wchar_t *path, BYTE *hash, DWORD cap);
/* How a catalog member is passed: 0 by path, 1 with an open handle, 2 with
 * the handle and its SHA-256 hash, 3 that with dwCatalogVersion 2, 4 with
 * the SHA-1 hash, 5 with the SHA-256 hash and a SHA-256 hCatAdmin. */
static int member_mode = 1;
static LONG trust_member(const wchar_t *cat, const wchar_t *tag, const wchar_t *member, DWORD revocation,
                         DWORD flags) {
    WINTRUST_CATALOG_INFO c;
    WINTRUST_DATA d;
    HANDLE h = INVALID_HANDLE_VALUE;
    BYTE hash[64];
    LONG r;
    memset(&c, 0, sizeof c);
    memset(&d, 0, sizeof d);
    c.cbStruct = sizeof c;
    c.pcwszCatalogFilePath = cat;
    c.pcwszMemberTag = tag;
    c.pcwszMemberFilePath = member;
    if (member_mode >= 1) {
        h = CreateFileW(member, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
        c.hMemberFile = h;
    }
    if (member_mode == 4) {
        c.cbCalculatedFileHash = file_hash(NULL, member, hash, sizeof hash);
        c.pbCalculatedFileHash = c.cbCalculatedFileHash ? hash : NULL;
    } else if (member_mode >= 2) {
        HCATADMIN admin;
        if (CryptCATAdminAcquireContext2(&admin, &driver, BCRYPT_SHA256_ALGORITHM, NULL, 0)) {
            c.cbCalculatedFileHash = file_hash(admin, member, hash, sizeof hash);
            c.pbCalculatedFileHash = c.cbCalculatedFileHash ? hash : NULL;
            if (member_mode == 5) {
                c.hCatAdmin = admin;
                c.dwCatalogVersion = 2;
                d.cbStruct = sizeof d;
            }
        }
        if (member_mode == 3) c.dwCatalogVersion = 2;
        if (member_mode == 5) {
            d.fdwRevocationChecks = revocation;
            d.dwUnionChoice = WTD_CHOICE_CATALOG;
            d.pCatalog = &c;
            d.dwProvFlags = flags;
            r = trust(&d);
            CryptCATAdminReleaseContext(admin, 0);
            if (h != INVALID_HANDLE_VALUE) CloseHandle(h);
            return r;
        }
        CryptCATAdminReleaseContext(admin, 0);
    }
    d.fdwRevocationChecks = revocation;
    d.dwUnionChoice = WTD_CHOICE_CATALOG;
    d.pCatalog = &c;
    d.dwProvFlags = flags;
    r = trust(&d);
    if (h != INVALID_HANDLE_VALUE) CloseHandle(h);
    return r;
}
#define CACHE WTD_CACHE_ONLY_URL_RETRIEVAL

/* A file's hash as a catalog names it, into `hash`; its length or 0. */
static DWORD file_hash(HCATADMIN admin, const wchar_t *path, BYTE *hash, DWORD cap) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    DWORD n = cap;
    BOOL ok;
    if (h == INVALID_HANDLE_VALUE) return 0;
    ok = admin ? CryptCATAdminCalcHashFromFileHandle2(admin, h, &n, hash, 0)
               : CryptCATAdminCalcHashFromFileHandle(h, &n, hash, 0);
    CloseHandle(h);
    return ok ? n : 0;
}
/* A hash as a member tag: upper-case hexadecimal. */
static void tag_of(const BYTE *hash, DWORD n, wchar_t *tag) {
    DWORD i;
    for (i = 0; i < n; i++) {
        tag[2 * i] = L"0123456789ABCDEF"[hash[i] >> 4];
        tag[2 * i + 1] = L"0123456789ABCDEF"[hash[i] & 15];
    }
    tag[2 * n] = 0;
}

static const wchar_t *const programs[] = {L"signed-good.exe", L"signed-revoked.exe", L"hello.exe",
                                          L"readme.txt", L"missing.exe"};
static void files_step(void) {
    int i;
    for (i = 0; i < 5; i++) {
        wput(programs[i]);
        put(" none=");
        hex32((unsigned long)trust_file(fixture(programs[i]), WTD_REVOKE_NONE, CACHE));
        put(" chain-cached=");
        hex32((unsigned long)trust_file(fixture(programs[i]), WTD_REVOKE_WHOLECHAIN, CACHE));
        nl();
    }
    {
        WINTRUST_DATA d;
        GUID unknown = {0x12345678, 0x1234, 0x1234, {1, 2, 3, 4, 5, 6, 7, 8}};
        memset(&d, 0, sizeof d);
        d.cbStruct = sizeof d;
        d.dwUIChoice = WTD_UI_NONE;
        d.dwUnionChoice = 9;
        put("bad-choice=");
        hex32((unsigned long)WinVerifyTrust(NULL, &generic, &d));
        put(" unknown-action=");
        hex32((unsigned long)WinVerifyTrust(NULL, &unknown, &d));
        nl();
    }
}
static void hashes_step(void) {
    static const wchar_t *const names[] = {L"hello.exe", L"signed-good.exe", L"readme.txt"};
    HCATADMIN sha1, sha256, sha384 = NULL;
    BYTE hash[64];
    DWORD n;
    int i;
    put("acquire");
    put(CryptCATAdminAcquireContext(&sha1, &driver, 0) ? " ok" : " failed");
    put(CryptCATAdminAcquireContext2(&sha256, &driver, BCRYPT_SHA256_ALGORITHM, NULL, 0) ? " ok" : " failed");
    SetLastError(0);
    if (CryptCATAdminAcquireContext2(&sha384, &driver, L"SHA384", NULL, 0)) {
        put(" sha384 ok");
        CryptCATAdminReleaseContext(sha384, 0);
    } else {
        put(" sha384 error=");
        hex32(GetLastError());
    }
    nl();
    for (i = 0; i < 3; i++) {
        wput(names[i]);
        n = file_hash(NULL, fixture(names[i]), hash, sizeof hash);
        put(" sha1=");
        bytes(hash, n);
        n = file_hash(sha256, fixture(names[i]), hash, sizeof hash);
        put(" sha256=");
        bytes(hash, n);
        nl();
    }
    {
        HANDLE h = CreateFileW(fixture(L"readme.txt"), GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
        BOOL ok;
        n = 4;
        SetLastError(0);
        ok = CryptCATAdminCalcHashFromFileHandle2(sha256, h, &n, hash, 0);
        put("short ok=");
        num(ok);
        put(" error=");
        hex32(ok ? 0 : GetLastError());
        put(" size=");
        num(n);
        n = 0;
        SetLastError(0);
        ok = CryptCATAdminCalcHashFromFileHandle(h, &n, NULL, 0);
        put(" query ok=");
        num(ok);
        put(" error=");
        hex32(ok ? 0 : GetLastError());
        put(" size=");
        num(n);
        CloseHandle(h);
        nl();
    }
    CryptCATAdminReleaseContext(sha1, 0);
    CryptCATAdminReleaseContext(sha256, 0);
}
static void catalog_step(void) {
    HCATADMIN admin;
    BYTE hash[64];
    wchar_t hello[130], readme[130], sha1tag[130];
    DWORD n;
    static wchar_t cat[MAX_PATH];
    const wchar_t *p = fixture(L"doors-trust-test.cat");
    for (n = 0; p[n]; n++) cat[n] = p[n];
    cat[n] = 0;
    CryptCATAdminAcquireContext2(&admin, &driver, BCRYPT_SHA256_ALGORITHM, NULL, 0);
    n = file_hash(admin, fixture(L"hello.exe"), hash, sizeof hash);
    tag_of(hash, n, hello);
    n = file_hash(admin, fixture(L"readme.txt"), hash, sizeof hash);
    tag_of(hash, n, readme);
    n = file_hash(NULL, fixture(L"hello.exe"), hash, sizeof hash);
    tag_of(hash, n, sha1tag);
    CryptCATAdminReleaseContext(admin, 0);
    for (member_mode = 0; member_mode < 6; member_mode++) {
        put("mode=");
        num(member_mode);
        put(" hello=");
        hex32((unsigned long)trust_member(cat, hello, fixture(L"hello.exe"), WTD_REVOKE_NONE, CACHE));
        put(" readme=");
        hex32((unsigned long)trust_member(cat, readme, fixture(L"readme.txt"), WTD_REVOKE_NONE, CACHE));
        put(" sha1-tag=");
        hex32((unsigned long)trust_member(cat, sha1tag, fixture(L"hello.exe"), WTD_REVOKE_NONE, CACHE));
        put(" signed-copy=");
        hex32((unsigned long)trust_member(cat, hello, fixture(L"signed-good.exe"), WTD_REVOKE_NONE, CACHE));
        nl();
        put("wrong-file=");
        hex32((unsigned long)trust_member(cat, hello, fixture(L"readme.txt"), WTD_REVOKE_NONE, CACHE));
        put(" unknown-tag=");
        hex32((unsigned long)trust_member(cat, L"00112233", fixture(L"hello.exe"), WTD_REVOKE_NONE, CACHE));
        put(" not-a-catalog=");
        hex32((unsigned long)trust_member(fixture(L"signed-good.exe"), hello, fixture(L"hello.exe"),
                                          WTD_REVOKE_NONE, CACHE));
        put(" missing-catalog=");
        hex32((unsigned long)trust_member(fixture(L"missing.cat"), hello, fixture(L"hello.exe"), WTD_REVOKE_NONE,
                                          CACHE));
        nl();
    }
    member_mode = 1;
}
/* Raw only (the owner's PC): a System32 file found in the system's catalogs. */
static void system_step(void) {
    HCATADMIN admin;
    HCATINFO info = NULL;
    BYTE hash[64];
    DWORD n;
    wchar_t tag[130];
    const wchar_t *file = L"C:\\Windows\\System32\\kernel32.dll";
    put("acquire=");
    num(CryptCATAdminAcquireContext2(&admin, &driver, BCRYPT_SHA256_ALGORITHM, NULL, 0));
    n = file_hash(admin, file, hash, sizeof hash);
    put(" hash-bytes=");
    num(n);
    info = CryptCATAdminEnumCatalogFromHash(admin, hash, n, 0, NULL);
    put(" found=");
    num(info != NULL);
    if (info) {
        CATALOG_INFO ci;
        memset(&ci, 0, sizeof ci);
        ci.cbStruct = sizeof ci;
        if (CryptCATCatalogInfoFromContext(info, &ci, 0)) {
            tag_of(hash, n, tag);
            put(" store=");
            num(wsuffix(ci.wszCatalogFile, L".cat"));
            put(" verified=");
            hex32((unsigned long)trust_member(ci.wszCatalogFile, tag, file, WTD_REVOKE_NONE, CACHE));
            put(" catalog=");
            wput(ci.wszCatalogFile);
        }
        CryptCATAdminReleaseCatalogContext(admin, info, 0);
    }
    CryptCATAdminReleaseContext(admin, 0);
    nl();
}

/* Guest only. */
static int failed_check, check;
static DWORD last;
#define EXPECT(c)                                    \
    do {                                             \
        check++;                                     \
        if (!(c)) {                                  \
            last = GetLastError();                   \
            if (!failed_check) failed_check = check; \
        }                                            \
    } while (0)
#define ROOT_KEY L"SOFTWARE\\Microsoft\\SystemCertificates\\ROOT\\Certificates\\C55DE7AB01EAA1FD641F754FDBF7BEF07D698EA4"
/* The test root into the machine Root store (as CertAddCertificateContextToStore
 * keeps it: CERT_CERT_PROP_ID with the DER). */
static int trust_root(void) {
    static BYTE blob[4096];
    HANDLE h = CreateFileW(fixture(L"root.der"), GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    DWORD n = 0, entry[3];
    HKEY key;
    LONG r;
    if (h == INVALID_HANDLE_VALUE) return 0;
    ReadFile(h, blob + 12, sizeof blob - 12, &n, NULL);
    CloseHandle(h);
    entry[0] = 32;
    entry[1] = 1;
    entry[2] = n;
    memcpy(blob, entry, 12);
    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, ROOT_KEY, 0, NULL, 0, KEY_ALL_ACCESS, NULL, &key, NULL)) return 0;
    r = RegSetValueExW(key, L"Blob", 0, REG_BINARY, blob, n + 12);
    RegCloseKey(key);
    return r == ERROR_SUCCESS;
}
static void trusted_step(void) {
    EXPECT(trust_file(fixture(L"signed-good.exe"), WTD_REVOKE_NONE, CACHE) == (LONG)CERT_E_CHAINING);
    EXPECT(trust_root());
    EXPECT(trust_file(fixture(L"signed-good.exe"), WTD_REVOKE_NONE, 0) == ERROR_SUCCESS);
    EXPECT(trust_file(fixture(L"signed-revoked.exe"), WTD_REVOKE_NONE, 0) == ERROR_SUCCESS);
    EXPECT(trust_file(fixture(L"hello.exe"), WTD_REVOKE_NONE, 0) == (LONG)TRUST_E_NOSIGNATURE);
}
static void revocation_step(void) {
    /* Nothing cached yet: cache-only retrieval cannot check. */
    EXPECT(trust_file(fixture(L"signed-good.exe"), WTD_REVOKE_WHOLECHAIN, CACHE) ==
           (LONG)CERT_E_REVOCATION_FAILURE);
    /* The CRL from the test host. */
    EXPECT(trust_file(fixture(L"signed-good.exe"), WTD_REVOKE_WHOLECHAIN, 0) == ERROR_SUCCESS);
    EXPECT(trust_file(fixture(L"signed-revoked.exe"), WTD_REVOKE_WHOLECHAIN, 0) == (LONG)CERT_E_REVOKED);
    /* Now cached. */
    EXPECT(trust_file(fixture(L"signed-revoked.exe"), WTD_REVOKE_WHOLECHAIN, CACHE) == (LONG)CERT_E_REVOKED);
    EXPECT(trust_file(fixture(L"signed-good.exe"), 0, WTD_REVOCATION_CHECK_CHAIN | CACHE) == ERROR_SUCCESS);
    EXPECT(trust_file(fixture(L"signed-revoked.exe"), WTD_REVOKE_WHOLECHAIN, WTD_REVOCATION_CHECK_NONE) ==
           ERROR_SUCCESS);
}
static void store_step(void) {
    HCATADMIN admin;
    HCATINFO info, again;
    BYTE hash[64];
    DWORD n;
    wchar_t tag[130];
    CATALOG_INFO ci;
    EXPECT(CryptCATAdminAcquireContext2(&admin, &driver, BCRYPT_SHA256_ALGORITHM, NULL, 0));
    n = file_hash(admin, fixture(L"hello.exe"), hash, sizeof hash);
    EXPECT(n == 32);
    tag_of(hash, n, tag);
    CryptCATAdminRemoveCatalog(admin, L"doors-trust-test.cat", 0);
    SetLastError(0);
    EXPECT(CryptCATAdminEnumCatalogFromHash(admin, hash, n, 0, NULL) == NULL);
    info = CryptCATAdminAddCatalog(admin, (PWSTR)fixture(L"doors-trust-test.cat"), L"doors-trust-test.cat", 0);
    EXPECT(info != NULL);
    if (info) CryptCATAdminReleaseCatalogContext(admin, info, 0);
    info = CryptCATAdminEnumCatalogFromHash(admin, hash, n, 0, NULL);
    EXPECT(info != NULL);
    memset(&ci, 0, sizeof ci);
    ci.cbStruct = sizeof ci;
    EXPECT(info && CryptCATCatalogInfoFromContext(info, &ci, 0));
    EXPECT(wsuffix(ci.wszCatalogFile,
                   L"\\System32\\CatRoot\\{F750E6C3-38EE-11D1-85E5-00C04FC295EE}\\doors-trust-test.cat"));
    EXPECT(trust_member(ci.wszCatalogFile, tag, fixture(L"hello.exe"), WTD_REVOKE_NONE, 0) == ERROR_SUCCESS);
    EXPECT(trust_member(ci.wszCatalogFile, tag, fixture(L"hello.exe"), WTD_REVOKE_WHOLECHAIN, 0) ==
           ERROR_SUCCESS);
    EXPECT(trust_member(ci.wszCatalogFile, tag, fixture(L"readme.txt"), WTD_REVOKE_NONE, 0) ==
           (LONG)TRUST_E_BAD_DIGEST);
    /* No second catalog names it. */
    again = info;
    SetLastError(0);
    EXPECT(CryptCATAdminEnumCatalogFromHash(admin, hash, n, 0, &again) == NULL &&
           GetLastError() == ERROR_NOT_FOUND);
    EXPECT(CryptCATAdminRemoveCatalog(admin, L"doors-trust-test.cat", 0));
    EXPECT(CryptCATAdminEnumCatalogFromHash(admin, hash, n, 0, NULL) == NULL);
    CryptCATAdminReleaseContext(admin, 0);
}
static void untrust_step(void) {
    EXPECT(RegDeleteKeyW(HKEY_LOCAL_MACHINE, ROOT_KEY) == ERROR_SUCCESS);
    EXPECT(trust_file(fixture(L"signed-good.exe"), WTD_REVOKE_NONE, CACHE) == (LONG)CERT_E_CHAINING);
}

struct step {
    const char *name;
    void (*run)(void);
    /* 0 compared, 1 guest only, 2 raw only. */
    int kind;
};
static const struct step steps[] = {
    {"files", files_step, 0},     {"hashes", hashes_step, 0},         {"catalog", catalog_step, 0},
    {"trusted", trusted_step, 1}, {"revocation", revocation_step, 1}, {"store", store_step, 1},
    {"untrust", untrust_step, 1}, {"system", system_step, 2},
};
static const char *const expected[] = {
#include "trustprobe.expected.h"
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[600];
    int n = line_length(s), k;
    if (n > 599) n = 599;
    for (k = 0; k < n; k++) b[k] = s[k];
    b[n] = 0;
    put(label);
    put(": ");
    put(b);
    put("\r\n");
}
static int compare(const char *got, const char *want, int *count, const char **g, const char **w) {
    int line = 1;
    *count = 0;
    for (;;) {
        int a = line_length(got), b = line_length(want), k;
        if (!*got && !*want) return 0;
        if (a != b) goto differ;
        for (k = 0; k < a; k++)
            if (got[k] != want[k]) goto differ;
        got += a;
        want += b;
        while (*got == '\r' || *got == '\n') got++;
        while (*want == '\r' || *want == '\n') want++;
        (*count)++;
        line++;
        continue;
    differ:
        *g = got;
        *w = want;
        return line;
    }
}
static int last_word_is(const wchar_t *line, const wchar_t *word) {
    const wchar_t *w = line, *p;
    for (p = line; *p; p++)
        if (*p == L' ' && p[1] && p[1] != L' ') w = p + 1;
    while (*w && *w == *word) w++, word++;
    return *w == *word;
}
void entry(void) {
    const wchar_t *line = GetCommandLineW();
    int raw = last_word_is(line, L"raw"), guest = last_word_is(line, L"guest"), failed = 0, i, compared = 0;
    DWORD n;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    n = GetModuleFileNameW(NULL, folder, MAX_PATH);
    for (i = 0; i < (int)n; i++)
        if (folder[i] == L'\\' || folder[i] == L'/') folder_length = i + 1;
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        if (steps[i].kind == 2) {
            if (!raw) continue;
            put("== ");
            put(steps[i].name);
            put(" (not compared)");
            nl();
            steps[i].run();
            continue;
        }
        if (steps[i].kind == 1) {
            if (!guest || failed) continue;
            check = 0;
            failed_check = 0;
            steps[i].run();
            put("TRUST-A ");
            num(i + 1);
            put(" ");
            put(steps[i].name);
            if (failed_check) {
                put(" FAILED check ");
                num(failed_check);
                put(" error=");
                hex32(last);
                failed = 1;
            } else {
                put(" ok ");
                num(check);
            }
            nl();
            continue;
        }
        if (raw) {
            put("== ");
            put(steps[i].name);
            nl();
            steps[i].run();
            continue;
        }
        {
            int count, at;
            const char *g = 0, *w = 0;
            captured_length = 0;
            captured[0] = 0;
            capturing = 1;
            steps[i].run();
            capturing = 0;
            at = compare(captured, expected[compared], &count, &g, &w);
            compared++;
            put("TRUST-A ");
            num(i + 1);
            put(" ");
            put(steps[i].name);
            if (at == 0) {
                put(" ok ");
                num(count);
                nl();
            } else {
                put(" FAILED line ");
                num(at);
                nl();
                put_line("got", g);
                put_line("expected", w);
                failed = 1;
            }
        }
    }
    if (raw) ExitProcess(0);
    put(failed ? "TRUST-A FAILED\r\n" : "TRUST-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

/* CRYPT-A probe (P16 M5): CryptoAPI keys from an ordinary Windows console
 * program (Windows SDK, no C runtime):
 *
 *   cryptprobe.exe raw | check | guest
 *
 * Every compared step works in CRYPT_VERIFYCONTEXT contexts only, whose
 * keys live in memory and are never saved: the providers' algorithm lists,
 * symmetric keys imported as PLAINTEXTKEYBLOBs (RC2, RC4, DES, two- and
 * three-key triple DES, AES) with their parameters and ciphertexts in each
 * mode, keys derived from hashes, an RSA key pair imported from a fixed
 * PRIVATEKEYBLOB (signatures, decryption of fixed ciphertexts, a
 * SIMPLEBLOB, blob export), HMAC, generated keys' sizes and the error
 * codes. Their output is compared with what Windows 11 printed (`raw` on the
 * owner's PC; `cryptprobe.expected.h`). `guest` also creates a key
 * container with a signature key, reopens and deletes it, which only the
 * guest may do. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <wincrypt.h>
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
static char captured[65536];
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
/* " ok" or " error=0x...". */
static void result(BOOL ok) {
    if (ok) {
        put(" ok");
    } else {
        put(" error=");
        hex32(GetLastError());
    }
}
static int same(const BYTE *a, const BYTE *b, DWORD n) {
    DWORD i;
    for (i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}
static DWORD unhex(const char *s, BYTE *outb, DWORD cap) {
    DWORD n = 0;
    while (s[0] && s[1] && n < cap) {
        int hi = s[0] <= '9' ? s[0] - '0' : s[0] - 'a' + 10;
        int lo = s[1] <= '9' ? s[1] - '0' : s[1] - 'a' + 10;
        outb[n++] = (BYTE)(hi * 16 + lo);
        s += 2;
    }
    return n;
}

#include "cryptprobe.fixture.h"

static const BYTE text[] = "Doors CryptoAPI test";  /* 20 bytes and a NUL */
static const BYTE key_bytes[32] = {0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba,
                                   0x98, 0x76, 0x54, 0x32, 0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
                                   0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff};

static HCRYPTPROV base, enhanced, strong, aes;
static const wchar_t *const provider_names[4] = {MS_DEF_PROV_W, MS_ENHANCED_PROV_W, MS_STRONG_PROV_W,
                                                 MS_ENH_RSA_AES_PROV_W};
static const char *const provider_labels[4] = {"base", "enhanced", "strong", "aes"};
static HCRYPTPROV *const providers[4] = {&base, &enhanced, &strong, &aes};
static void open_providers(void) {
    int i;
    for (i = 0; i < 4; i++)
        if (!*providers[i])
            CryptAcquireContextW(providers[i], NULL, provider_names[i], i == 3 ? PROV_RSA_AES : PROV_RSA_FULL,
                                 CRYPT_VERIFYCONTEXT | CRYPT_SILENT);
}

/* PLAINTEXTKEYBLOB of `len` key bytes for `alg`. */
static BOOL import_plain(HCRYPTPROV p, ALG_ID alg, DWORD len, DWORD flags, HCRYPTKEY *key) {
    BYTE blob[sizeof(BLOBHEADER) + 4 + 32];
    BLOBHEADER *h = (BLOBHEADER *)blob;
    h->bType = PLAINTEXTKEYBLOB;
    h->bVersion = CUR_BLOB_VERSION;
    h->reserved = 0;
    h->aiKeyAlg = alg;
    *(DWORD *)(blob + sizeof(BLOBHEADER)) = len;
    memcpy(blob + sizeof(BLOBHEADER) + 4, key_bytes, len);
    return CryptImportKey(p, blob, sizeof(BLOBHEADER) + 4 + len, 0, flags, key);
}
static void key_dword(HCRYPTKEY k, DWORD param, const char *label) {
    DWORD v = 0, n = sizeof v;
    put(" ");
    put(label);
    put("=");
    if (CryptGetKeyParam(k, param, (BYTE *)&v, &n, 0))
        num(v);
    else {
        put("error:");
        hex32(GetLastError());
    }
}
static void key_bytes_param(HCRYPTKEY k, DWORD param, const char *label) {
    BYTE b[64];
    DWORD n = sizeof b;
    put(" ");
    put(label);
    put("=");
    if (CryptGetKeyParam(k, param, b, &n, 0)) {
        num(n);
        put(":");
        bytes(b, n);
    } else {
        put("error:");
        hex32(GetLastError());
    }
}
/* Encrypts `text` (20 bytes) in one final call; prints the ciphertext and
 * whether decrypting it gives `text` back. */
static void round_trip(HCRYPTKEY k) {
    BYTE b[64];
    DWORD n = 20;
    memcpy(b, text, 20);
    put(" enc");
    if (CryptEncrypt(k, 0, TRUE, 0, b, &n, sizeof b)) {
        put("=");
        bytes(b, n);
        put(" dec");
        if (CryptDecrypt(k, 0, TRUE, 0, b, &n))
            put(n == 20 && same(b, text, 20) ? "=same" : "=differs");
        else
            result(FALSE);
    } else
        result(FALSE);
}

static void providers_step(void) {
    int i;
    open_providers();
    for (i = 0; i < 4; i++) {
        PROV_ENUMALGS_EX a;
        PROV_ENUMALGS b;
        DWORD n = sizeof a, flags = CRYPT_FIRST;
        while (CryptGetProvParam(*providers[i], PP_ENUMALGS_EX, (BYTE *)&a, &n, flags)) {
            put(provider_labels[i]);
            put(" ");
            hex32(a.aiAlgid);
            put("/");
            num(a.dwDefaultLen);
            put("/");
            num(a.dwMinLen);
            put("/");
            num(a.dwMaxLen);
            put("/");
            hex32(a.dwProtocols);
            put("/");
            put(a.szName);
            put("/");
            put(a.szLongName);
            nl();
            flags = 0;
            n = sizeof a;
        }
        /* PP_ENUMALGS: the shorter record, and its end. */
        n = sizeof b;
        flags = CRYPT_FIRST;
        put(provider_labels[i]);
        put(" short");
        while (CryptGetProvParam(*providers[i], PP_ENUMALGS, (BYTE *)&b, &n, flags)) {
            put(" ");
            hex32(b.aiAlgid);
            put("/");
            num(b.dwBitLen);
            put("/");
            put(b.szName);
            flags = 0;
            n = sizeof b;
        }
        put(" end=");
        hex32(GetLastError());
        nl();
    }
}

struct sym {
    ALG_ID alg;
    DWORD len;
    const char *name;
};
static const struct sym syms[] = {
    {CALG_RC2, 5, "rc2-40"},       {CALG_RC2, 16, "rc2-128"},  {CALG_RC4, 5, "rc4-40"},
    {CALG_RC4, 16, "rc4-128"},     {CALG_DES, 8, "des"},       {CALG_3DES_112, 16, "3des-112"},
    {CALG_3DES, 24, "3des"},       {CALG_AES_128, 16, "aes-128"}, {CALG_AES_192, 24, "aes-192"},
    {CALG_AES_256, 32, "aes-256"},
};
static void import_step(void) {
    int i, s;
    open_providers();
    for (i = 0; i < 4; i++) {
        if (i == 2) continue; /* strong = enhanced */
        for (s = 0; s < (int)(sizeof syms / sizeof syms[0]); s++) {
            HCRYPTKEY k;
            put(provider_labels[i]);
            put(" ");
            put(syms[s].name);
            if (!import_plain(*providers[i], syms[s].alg, syms[s].len, 0, &k)) {
                result(FALSE);
                nl();
                continue;
            }
            key_dword(k, KP_KEYLEN, "len");
            key_dword(k, KP_BLOCKLEN, "block");
            if (syms[s].alg == CALG_RC2) key_dword(k, KP_EFFECTIVE_KEYLEN, "effective");
            if (syms[s].alg != CALG_RC4) {
                key_dword(k, KP_MODE, "mode");
                key_dword(k, KP_PADDING, "padding");
                key_bytes_param(k, KP_IV, "iv");
            }
            key_bytes_param(k, KP_SALT, "salt");
            key_dword(k, KP_PERMISSIONS, "permissions");
            round_trip(k);
            nl();
            CryptDestroyKey(k);
        }
    }
}
static void modes_step(void) {
    static const DWORD modes[] = {CRYPT_MODE_ECB, CRYPT_MODE_CBC, CRYPT_MODE_CFB, CRYPT_MODE_OFB, CRYPT_MODE_CTS};
    static const char *const mode_names[] = {"ecb", "cbc", "cfb", "ofb", "cts"};
    static const ALG_ID algs[] = {CALG_3DES, CALG_AES_128, CALG_RC2, CALG_DES};
    static const DWORD lens[] = {24, 16, 16, 8};
    static const char *const names[] = {"3des", "aes-128", "rc2-128", "des"};
    static const BYTE iv[16] = {0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
                                0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf};
    int a, m;
    open_providers();
    for (a = 0; a < 4; a++) {
        for (m = 0; m < 5; m++) {
            HCRYPTKEY k;
            BOOL set;
            put(names[a]);
            put(" ");
            put(mode_names[m]);
            if (!import_plain(aes, algs[a], lens[a], 0, &k)) {
                result(FALSE);
                nl();
                continue;
            }
            set = CryptSetKeyParam(k, KP_MODE, (const BYTE *)&modes[m], 0);
            put(" set");
            result(set);
            if (set) {
                put(" iv");
                result(CryptSetKeyParam(k, KP_IV, iv, 0));
                round_trip(k);
            }
            nl();
            CryptDestroyKey(k);
        }
    }
    /* Pieces: two blocks not final, then the rest final; a decryption
     * whose padding is wrong; a buffer too small. */
    {
        HCRYPTKEY k;
        BYTE b[64];
        DWORD n;
        import_plain(aes, CALG_AES_128, 16, 0, &k);
        memcpy(b, text, 20);
        n = 16;
        put("pieces first");
        result(CryptEncrypt(k, 0, FALSE, 0, b, &n, sizeof b));
        put(" n=");
        num(n);
        n = 4;
        put(" last");
        result(CryptEncrypt(k, 0, TRUE, 0, b + 16, &n, sizeof b - 16));
        put(" n=");
        num(n);
        put(" out=");
        bytes(b, 16 + n);
        nl();
        n = 20;
        memcpy(b, text, 20);
        put("small");
        result(CryptEncrypt(k, 0, TRUE, 0, b, &n, 20));
        put(" needed=");
        num(n);
        n = 20;
        put(" length-only");
        result(CryptEncrypt(k, 0, TRUE, 0, NULL, &n, 0));
        put(" n=");
        num(n);
        nl();
        memcpy(b, text, 16);
        n = 16;
        put("bad-padding");
        result(CryptDecrypt(k, 0, TRUE, 0, b, &n));
        n = 15;
        put(" bad-length");
        result(CryptDecrypt(k, 0, TRUE, 0, b, &n));
        nl();
        CryptDestroyKey(k);
    }
}
static void derive_step(void) {
    static const ALG_ID hashes[] = {CALG_MD5, CALG_SHA1, CALG_SHA_256};
    static const char *const hash_names[] = {"md5", "sha1", "sha256"};
    static const ALG_ID algs[] = {CALG_RC4, CALG_RC2, CALG_DES, CALG_3DES_112, CALG_3DES, CALG_AES_128, CALG_AES_256};
    static const char *const names[] = {"rc4", "rc2", "des", "3des-112", "3des", "aes-128", "aes-256"};
    static const DWORD lengths[] = {0, 40, 56, 128};
    int p, h, a, l;
    open_providers();
    for (p = 0; p < 4; p++) {
        if (p == 2) continue;
        for (h = 0; h < 3; h++) {
            for (a = 0; a < 7; a++) {
                for (l = 0; l < 4; l++) {
                    HCRYPTHASH hh;
                    HCRYPTKEY k;
                    BYTE blob[128];
                    DWORD n = sizeof blob;
                    if (l > 0 && algs[a] != CALG_RC4 && algs[a] != CALG_RC2) continue;
                    if (!CryptCreateHash(*providers[p], hashes[h], 0, 0, &hh)) continue;
                    CryptHashData(hh, (const BYTE *)"password", 8, 0);
                    put(provider_labels[p]);
                    put(" ");
                    put(hash_names[h]);
                    put(" ");
                    put(names[a]);
                    put("/");
                    num(lengths[l]);
                    if (CryptDeriveKey(*providers[p], algs[a], hh, CRYPT_EXPORTABLE | (lengths[l] << 16), &k)) {
                        key_dword(k, KP_KEYLEN, "len");
                        key_bytes_param(k, KP_SALT, "salt");
                        put(" key=");
                        if (CryptExportKey(k, 0, PLAINTEXTKEYBLOB, 0, blob, &n))
                            bytes(blob + sizeof(BLOBHEADER) + 4, *(DWORD *)(blob + sizeof(BLOBHEADER)));
                        else
                            result(FALSE);
                        round_trip(k);
                        CryptDestroyKey(k);
                    } else
                        result(FALSE);
                    nl();
                    CryptDestroyHash(hh);
                }
            }
        }
    }
    /* The hash is finished by the derivation. */
    {
        HCRYPTHASH hh;
        HCRYPTKEY k;
        CryptCreateHash(enhanced, CALG_SHA1, 0, 0, &hh);
        CryptHashData(hh, (const BYTE *)"password", 8, 0);
        CryptDeriveKey(enhanced, CALG_RC4, hh, 0, &k);
        put("after-derive hash-data");
        result(CryptHashData(hh, (const BYTE *)"x", 1, 0));
        put(" export");
        {
            BYTE blob[64];
            DWORD n = sizeof blob;
            result(CryptExportKey(k, 0, PLAINTEXTKEYBLOB, 0, blob, &n));
        }
        nl();
        CryptDestroyKey(k);
        CryptDestroyHash(hh);
    }
}

static BYTE private_blob[sizeof(BLOBHEADER) + 600];
static DWORD private_length;
static void rsa_step(void) {
    HCRYPTKEY k, sk;
    BYTE b[600], c[256];
    DWORD n;
    static const ALG_ID hashes[] = {CALG_MD5, CALG_SHA1, CALG_SHA_256, CALG_SHA_512};
    static const char *const hash_names[] = {"md5", "sha1", "sha256", "sha512"};
    int h;
    open_providers();
    {
        BLOBHEADER *bh = (BLOBHEADER *)private_blob;
        bh->bType = PRIVATEKEYBLOB;
        bh->bVersion = CUR_BLOB_VERSION;
        bh->reserved = 0;
        bh->aiKeyAlg = CALG_RSA_KEYX;
        private_length = sizeof(BLOBHEADER) + unhex(private_hex, private_blob + sizeof(BLOBHEADER), 600);
    }
    put("import");
    result(CryptImportKey(aes, private_blob, private_length, 0, CRYPT_EXPORTABLE, &k));
    key_dword(k, KP_ALGID, "alg");
    key_dword(k, KP_KEYLEN, "len");
    key_dword(k, KP_BLOCKLEN, "block");
    key_dword(k, KP_PERMISSIONS, "permissions");
    nl();
    n = sizeof b;
    put("public");
    if (CryptExportKey(k, 0, PUBLICKEYBLOB, 0, b, &n)) {
        put(" n=");
        num(n);
        put(" blob=");
        bytes(b, n);
    } else
        result(FALSE);
    nl();
    n = sizeof b;
    put("private");
    if (CryptExportKey(k, 0, PRIVATEKEYBLOB, 0, b, &n)) {
        put(" n=");
        num(n);
        put(n == private_length && same(b, private_blob, n) ? " same" : " differs");
    } else
        result(FALSE);
    nl();
    for (h = 0; h < 4; h++) {
        HCRYPTHASH hh;
        BYTE sig[256];
        DWORD sn = sizeof sig;
        put("sign ");
        put(hash_names[h]);
        CryptCreateHash(aes, hashes[h], 0, 0, &hh);
        CryptHashData(hh, (const BYTE *)"Doors", 5, 0);
        if (CryptSignHashW(hh, AT_KEYEXCHANGE, NULL, 0, sig, &sn)) {
            put(" sig=");
            bytes(sig, sn);
            put(" verify");
            result(CryptVerifySignatureW(hh, sig, sn, k, NULL, 0));
            sig[5] ^= 1;
            put(" altered");
            result(CryptVerifySignatureW(hh, sig, sn, k, NULL, 0));
        } else
            result(FALSE);
        if (hashes[h] == CALG_SHA1) {
            sn = sizeof sig;
            put(" nooid");
            if (CryptSignHashW(hh, AT_KEYEXCHANGE, NULL, CRYPT_NOHASHOID, sig, &sn)) {
                put("=");
                bytes(sig, sn);
            } else
                result(FALSE);
        }
        nl();
        CryptDestroyHash(hh);
    }
    n = unhex(cipher_hex, c, sizeof c);
    put("decrypt");
    if (CryptDecrypt(k, 0, TRUE, 0, c, &n)) {
        put(" n=");
        num(n);
        put(" text=");
        bytes(c, n);
    } else
        result(FALSE);
    n = unhex(oaep_hex, c, sizeof c);
    put(" oaep");
    if (CryptDecrypt(k, 0, TRUE, CRYPT_OAEP, c, &n)) {
        put("=");
        bytes(c, n);
    } else
        result(FALSE);
    n = unhex(cipher_hex, c, sizeof c);
    c[3] ^= 0x40;
    put(" altered");
    result(CryptDecrypt(k, 0, TRUE, 0, c, &n));
    nl();
    /* Encrypting to the public key is randomized: its length and the
     * round trip. */
    memcpy(b, text, 20);
    n = 20;
    put("encrypt");
    result(CryptEncrypt(k, 0, TRUE, 0, b, &n, sizeof b));
    put(" n=");
    num(n);
    put(" back");
    result(CryptDecrypt(k, 0, TRUE, 0, b, &n));
    put(n == 20 && same(b, text, 20) ? " same" : " differs");
    n = 118;
    put(" too-long");
    result(CryptEncrypt(k, 0, TRUE, 0, b, &n, sizeof b));
    nl();
    /* A SIMPLEBLOB: an RC4 key encrypted to this key. */
    {
        BYTE blob[sizeof(BLOBHEADER) + 4 + 128];
        BLOBHEADER *bh = (BLOBHEADER *)blob;
        bh->bType = SIMPLEBLOB;
        bh->bVersion = CUR_BLOB_VERSION;
        bh->reserved = 0;
        bh->aiKeyAlg = CALG_RC4;
        *(ALG_ID *)(blob + sizeof(BLOBHEADER)) = CALG_RSA_KEYX;
        unhex(simple_hex, blob + sizeof(BLOBHEADER) + 4, 128);
        put("simpleblob");
        if (CryptImportKey(aes, blob, sizeof blob, k, CRYPT_EXPORTABLE, &sk)) {
            key_dword(sk, KP_KEYLEN, "len");
            round_trip(sk);
            n = sizeof b;
            put(" plain=");
            if (CryptExportKey(sk, 0, PLAINTEXTKEYBLOB, 0, b, &n))
                bytes(b + sizeof(BLOBHEADER) + 4, *(DWORD *)(b + sizeof(BLOBHEADER)));
            else
                result(FALSE);
            n = sizeof b;
            put(" export-simple");
            result(CryptExportKey(sk, k, SIMPLEBLOB, 0, b, &n));
            put(" n=");
            num(n);
            CryptDestroyKey(sk);
        } else
            result(FALSE);
        nl();
    }
    /* The public key alone. */
    {
        HCRYPTKEY pk;
        HCRYPTHASH hh;
        BYTE sig[256];
        DWORD sn = sizeof sig;
        n = sizeof b;
        CryptExportKey(k, 0, PUBLICKEYBLOB, 0, b, &n);
        put("public-only");
        result(CryptImportKey(enhanced, b, n, 0, 0, &pk));
        CryptCreateHash(enhanced, CALG_SHA1, 0, 0, &hh);
        CryptHashData(hh, (const BYTE *)"Doors", 5, 0);
        CryptSignHashW(hh, AT_KEYEXCHANGE, NULL, 0, sig, &sn);
        put(" sign-no-key");
        result(sn == sizeof sig ? FALSE : TRUE);
        put(" export-private");
        n = sizeof b;
        result(CryptExportKey(pk, 0, PRIVATEKEYBLOB, 0, b, &n));
        put(" decrypt");
        n = unhex(cipher_hex, c, sizeof c);
        result(CryptDecrypt(pk, 0, TRUE, 0, c, &n));
        nl();
        CryptDestroyHash(hh);
        CryptDestroyKey(pk);
    }
    CryptDestroyKey(k);
}
static void hmac_step(void) {
    static const ALG_ID hashes[] = {CALG_MD5, CALG_SHA1, CALG_SHA_256, CALG_SHA_384, CALG_SHA_512};
    static const char *const names[] = {"md5", "sha1", "sha256", "sha384", "sha512"};
    int h;
    open_providers();
    for (h = 0; h < 5; h++) {
        HCRYPTKEY k;
        HCRYPTHASH hh;
        HMAC_INFO info;
        BYTE blob[sizeof(BLOBHEADER) + 4 + 4], v[64];
        DWORD n = sizeof v;
        BLOBHEADER *bh = (BLOBHEADER *)blob;
        bh->bType = PLAINTEXTKEYBLOB;
        bh->bVersion = CUR_BLOB_VERSION;
        bh->reserved = 0;
        bh->aiKeyAlg = CALG_RC2;
        *(DWORD *)(blob + sizeof(BLOBHEADER)) = 4;
        memcpy(blob + sizeof(BLOBHEADER) + 4, "Jefe", 4);
        put("hmac ");
        put(names[h]);
        put(" import");
        result(CryptImportKey(aes, blob, sizeof blob, 0, CRYPT_IPSEC_HMAC_KEY, &k));
        memset(&info, 0, sizeof info);
        info.HashAlgid = hashes[h];
        CryptCreateHash(aes, CALG_HMAC, k, 0, &hh);
        put(" info");
        result(CryptSetHashParam(hh, HP_HMAC_INFO, (const BYTE *)&info, 0));
        CryptHashData(hh, (const BYTE *)"what do ya want for nothing?", 28, 0);
        put(" value");
        if (CryptGetHashParam(hh, HP_HASHVAL, v, &n, 0)) {
            put("=");
            bytes(v, n);
        } else
            result(FALSE);
        nl();
        CryptDestroyHash(hh);
        CryptDestroyKey(k);
    }
    {
        HCRYPTKEY k;
        HCRYPTHASH hh;
        put("hmac short-key");
        result(import_plain(aes, CALG_RC2, 4, 0, &k));
        put(" no-key");
        result(CryptCreateHash(aes, CALG_HMAC, 0, 0, &hh));
        nl();
    }
}
/* Session-key hashing, salt and effective length changes, duplicated keys
 * mid-message, the IV after a piece. */
static void extras_step(void) {
    open_providers();
    {
        HCRYPTKEY k;
        HCRYPTHASH hh;
        BYTE v[64];
        DWORD n = sizeof v;
        import_plain(aes, CALG_AES_128, 16, 0, &k);
        CryptCreateHash(aes, CALG_SHA1, 0, 0, &hh);
        put("session-hash");
        result(CryptHashSessionKey(hh, k, 0));
        if (CryptGetHashParam(hh, HP_HASHVAL, v, &n, 0)) {
            put(" value=");
            bytes(v, n);
        }
        CryptDestroyHash(hh);
        n = sizeof v;
        CryptCreateHash(aes, CALG_SHA1, 0, 0, &hh);
        put(" little-endian");
        result(CryptHashSessionKey(hh, k, CRYPT_LITTLE_ENDIAN));
        if (CryptGetHashParam(hh, HP_HASHVAL, v, &n, 0)) {
            put(" value=");
            bytes(v, n);
        }
        CryptDestroyHash(hh);
        nl();
        CryptDestroyKey(k);
    }
    {
        HCRYPTKEY k;
        static const BYTE salt[11] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
        import_plain(enhanced, CALG_RC4, 5, 0, &k);
        put("salt set");
        result(CryptSetKeyParam(k, KP_SALT, salt, 0));
        key_bytes_param(k, KP_SALT, "salt");
        round_trip(k);
        CryptDestroyKey(k);
        import_plain(enhanced, CALG_RC4, 16, 0, &k);
        put(" rc4-128-salt");
        result(CryptSetKeyParam(k, KP_SALT, salt, 0));
        CryptDestroyKey(k);
        nl();
    }
    {
        HCRYPTKEY k;
        DWORD bits = 64;
        import_plain(enhanced, CALG_RC2, 16, 0, &k);
        put("effective set");
        result(CryptSetKeyParam(k, KP_EFFECTIVE_KEYLEN, (const BYTE *)&bits, 0));
        key_dword(k, KP_EFFECTIVE_KEYLEN, "effective");
        round_trip(k);
        bits = 2000;
        put(" too-large");
        result(CryptSetKeyParam(k, KP_EFFECTIVE_KEYLEN, (const BYTE *)&bits, 0));
        nl();
        CryptDestroyKey(k);
    }
    {
        /* CALG_MAC (a block cipher's CBC-MAC) and SSL3 SHAMD5. */
        static const ALG_ID algs[] = {CALG_DES, CALG_3DES, CALG_AES_128, CALG_RC4};
        static const DWORD lens[] = {8, 24, 16, 16};
        static const char *const names[] = {"des", "3des", "aes-128", "rc4"};
        int a;
        for (a = 0; a < 4; a++) {
            HCRYPTKEY k;
            HCRYPTHASH hh;
            BYTE v[64];
            DWORD n = sizeof v;
            import_plain(aes, algs[a], lens[a], 0, &k);
            put("mac ");
            put(names[a]);
            if (CryptCreateHash(aes, CALG_MAC, k, 0, &hh)) {
                CryptHashData(hh, text, 20, 0);
                put(" value");
                if (CryptGetHashParam(hh, HP_HASHVAL, v, &n, 0)) {
                    put("=");
                    bytes(v, n);
                } else
                    result(FALSE);
                CryptDestroyHash(hh);
            } else
                result(FALSE);
            nl();
            CryptDestroyKey(k);
        }
        {
            HCRYPTHASH hh;
            BYTE v[64];
            DWORD n = sizeof v, size = 0, sl = sizeof size;
            put("ssl3-shamd5 create");
            result(CryptCreateHash(aes, CALG_SSL3_SHAMD5, 0, 0, &hh));
            CryptGetHashParam(hh, HP_HASHSIZE, (BYTE *)&size, &sl, 0);
            put(" size=");
            num(size);
            put(" data");
            result(CryptHashData(hh, text, 20, 0));
            put(" value");
            if (CryptGetHashParam(hh, HP_HASHVAL, v, &n, 0)) {
                put("=");
                bytes(v, n);
            } else
                result(FALSE);
            nl();
            CryptDestroyHash(hh);
        }
    }
    {
        HCRYPTKEY k, d;
        BYTE a[64], b[64];
        DWORD n = 16;
        import_plain(aes, CALG_AES_128, 16, 0, &k);
        memcpy(a, text, 20);
        CryptEncrypt(k, 0, FALSE, 0, a, &n, sizeof a);
        put("duplicate");
        result(CryptDuplicateKey(k, NULL, 0, &d));
        key_bytes_param(k, KP_IV, "iv-after-piece");
        memcpy(b, a + 16, 4);
        memcpy(a + 16, text + 16, 4);
        memcpy(b, text + 16, 4);
        n = 4;
        CryptEncrypt(k, 0, TRUE, 0, a + 16, &n, sizeof a - 16);
        put(" original=");
        bytes(a + 16, n);
        n = 4;
        CryptEncrypt(d, 0, TRUE, 0, b, &n, sizeof b);
        put(" copy=");
        bytes(b, n);
        nl();
        CryptDestroyKey(d);
        CryptDestroyKey(k);
    }
}
static void genkey_step(void) {
    static const ALG_ID algs[] = {CALG_RC2, CALG_RC4, CALG_DES, CALG_3DES, CALG_AES_256, CALG_RSA_KEYX, CALG_RSA_SIGN};
    static const char *const names[] = {"rc2", "rc4", "des", "3des", "aes-256", "rsa-keyx", "rsa-sign"};
    int p, a;
    open_providers();
    for (p = 0; p < 4; p++) {
        if (p == 2) continue;
        put(provider_labels[p]);
        for (a = 0; a < 7; a++) {
            HCRYPTKEY k;
            put(" ");
            put(names[a]);
            if (CryptGenKey(*providers[p], algs[a], 0, &k)) {
                DWORD v = 0, n = sizeof v;
                CryptGetKeyParam(k, KP_KEYLEN, (BYTE *)&v, &n, 0);
                put("=");
                num(v);
                CryptDestroyKey(k);
            } else {
                put("=");
                hex32(GetLastError());
            }
        }
        nl();
    }
    {
        HCRYPTKEY k;
        BYTE blob[64];
        DWORD n = sizeof blob;
        put("errors rc4-256");
        result(CryptGenKey(enhanced, CALG_RC4, 256 << 16, &k));
        put(" rc4-32");
        result(CryptGenKey(enhanced, CALG_RC4, 32 << 16, &k));
        put(" aes-on-base");
        result(CryptGenKey(base, CALG_AES_128, 0, &k));
        put(" rsa-512");
        {
            BOOL made = CryptGenKey(enhanced, CALG_RSA_SIGN, 512 << 16, &k);
            result(made);
            if (made) CryptDestroyKey(k);
        }
        put(" user-key");
        result(CryptGetUserKey(enhanced, AT_KEYEXCHANGE, &k));
        CryptGenKey(enhanced, CALG_RC4, 0, &k);
        put(" not-exportable");
        result(CryptExportKey(k, 0, PLAINTEXTKEYBLOB, 0, blob, &n));
        CryptDestroyKey(k);
        put(" destroyed");
        result(CryptDestroyKey(k));
        nl();
    }
}

/* Guest only: a key container with a signature key, written to the
 * profile: generated, reopened, used and deleted. */
static int failed_check, check;
static DWORD last;
#define EXPECT(c)                         \
    do {                                  \
        check++;                          \
        if (!(c)) {                       \
            last = GetLastError();        \
            if (!failed_check) failed_check = check; \
        }                                 \
    } while (0)
static void container_step(void) {
    HCRYPTPROV p;
    HCRYPTKEY k;
    HCRYPTHASH hh;
    BYTE sig[256];
    DWORD sn = sizeof sig;
    const wchar_t *name = L"DoorsCryptProbe";
    CryptAcquireContextW(&p, name, MS_ENHANCED_PROV_W, PROV_RSA_FULL, CRYPT_DELETEKEYSET);
    EXPECT(CryptAcquireContextW(&p, name, MS_ENHANCED_PROV_W, PROV_RSA_FULL, CRYPT_NEWKEYSET));
    EXPECT(!CryptGetUserKey(p, AT_SIGNATURE, &k) && GetLastError() == (DWORD)NTE_NO_KEY);
    EXPECT(CryptGenKey(p, AT_SIGNATURE, 1024 << 16, &k));
    CryptDestroyKey(k);
    CryptReleaseContext(p, 0);
    EXPECT(CryptAcquireContextW(&p, name, MS_ENHANCED_PROV_W, PROV_RSA_FULL, 0));
    EXPECT(CryptGetUserKey(p, AT_SIGNATURE, &k));
    EXPECT(CryptCreateHash(p, CALG_SHA1, 0, 0, &hh));
    EXPECT(CryptHashData(hh, (const BYTE *)"Doors", 5, 0));
    EXPECT(CryptSignHashW(hh, AT_SIGNATURE, NULL, 0, sig, &sn) && sn == 128);
    EXPECT(CryptVerifySignatureW(hh, sig, sn, k, NULL, 0));
    CryptDestroyHash(hh);
    CryptDestroyKey(k);
    CryptReleaseContext(p, 0);
    EXPECT(CryptAcquireContextW(&p, name, MS_ENHANCED_PROV_W, PROV_RSA_FULL, CRYPT_DELETEKEYSET));
    EXPECT(!CryptAcquireContextW(&p, name, MS_ENHANCED_PROV_W, PROV_RSA_FULL, 0) &&
           GetLastError() == (DWORD)NTE_BAD_KEYSET);
}

struct step {
    const char *name;
    void (*run)(void);
    int guest_only;
};
static const struct step steps[] = {
    {"providers", providers_step, 0}, {"import", import_step, 0}, {"modes", modes_step, 0},
    {"derive", derive_step, 0},       {"rsa", rsa_step, 0},       {"hmac", hmac_step, 0},
    {"extras", extras_step, 0},       {"genkey", genkey_step, 0}, {"container", container_step, 1},
};
static const char *const expected[] = {
#include "cryptprobe.expected.h"
};
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static void put_line(const char *label, const char *s) {
    char b[1100];
    int n = line_length(s), k;
    if (n > 1099) n = 1099;
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
    int raw = last_word_is(line, L"raw"), guest = last_word_is(line, L"guest"), failed = 0, i;
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        if (steps[i].guest_only) {
            if (!guest || failed) continue;
            check = 0;
            failed_check = 0;
            steps[i].run();
            put("CRYPT-A ");
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
            at = compare(captured, expected[i], &count, &g, &w);
            put("CRYPT-A ");
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
    put(failed ? "CRYPT-A FAILED\r\n" : "CRYPT-A PASSED\r\n");
    ExitProcess(failed ? 1 : 0);
}

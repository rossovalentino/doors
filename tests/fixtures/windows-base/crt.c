/* Ordinary public C ABI imports, compiled separately with intrinsics disabled. */
#include <windows.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <errno.h>
void exercise_crt(void) {
    unsigned char *bytes = (unsigned char *)calloc(64, 1);
    unsigned char *grown;
    DWORD i;
    if (!bytes) ExitProcess(190);
    for (i=0;i<64;++i) if (bytes[i]) ExitProcess(191);
    memset(bytes,0x33,64);
    memcpy(bytes,"door-crt",9);
    if (strlen((const char *)bytes)!=8 || wcslen(L"door-wide")!=9 || memcmp(bytes,"door-crt",9)!=0 || abs(-73)!=73) ExitProcess(192);
    memmove(bytes+1,bytes,8);
    if (memcmp(bytes,"ddoor-crt",9)!=0) ExitProcess(193);
    memmove(bytes,bytes+1,8);
    grown=(unsigned char *)realloc(bytes,8192);
    if (!grown || memcmp(grown,"door-crt",8)!=0) ExitProcess(194);
    free(grown);
    free(NULL);
    bytes=(unsigned char *)malloc(32);
    if (!bytes) ExitProcess(195);
    memcpy(bytes,"still-owned",12);
    {
        volatile size_t impossible=(size_t)-1;
        errno=0;
        if (realloc(bytes,impossible)!=NULL || errno!=ENOMEM || memcmp(bytes,"still-owned",12)!=0) ExitProcess(196);
        errno=0;
        if (calloc(2,impossible)!=NULL || errno!=ENOMEM) ExitProcess(197);
    }
    free(bytes);
}

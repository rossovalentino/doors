/* INSTALL-A: a DLL custom action (caprobe.dll, in the Doors Tools
 * package's Binary table). Windows Installer loads it and calls
 * CheckMachine with the session's handle: it reads ProductName, sets
 * DOORS_CA_CHECK (the package writes it to the registry later) and logs a
 * message through MsiProcessMessage. No C runtime, no entry point. */
#define WIN32_LEAN_AND_MEAN
#define UNICODE
#include <windows.h>
#include <msi.h>
#include <msiquery.h>

static int same(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}

UINT __stdcall CheckMachine(MSIHANDLE session) {
    wchar_t name[64];
    DWORD length = 64;
    if (MsiGetPropertyW(session, L"ProductName", name, &length) != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;
    if (!same(name, L"Doors Tools") || length != 11) return ERROR_INSTALL_FAILURE;
    /* A buffer too small reports the length and ERROR_MORE_DATA. */
    wchar_t small[4];
    DWORD small_length = 4;
    if (MsiGetPropertyW(session, L"ProductName", small, &small_length) != ERROR_MORE_DATA || small_length != 11)
        return ERROR_INSTALL_FAILURE;
    if (MsiSetPropertyW(session, L"DOORS_CA_CHECK", L"passed") != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;
    MSIHANDLE record = MsiCreateRecord(1);
    MsiRecordSetStringW(record, 0, L"CaProbe: [1] checked");
    MsiRecordSetStringW(record, 1, name);
    MsiProcessMessage(session, INSTALLMESSAGE_INFO, record);
    MsiCloseHandle(record);
    return ERROR_SUCCESS;
}

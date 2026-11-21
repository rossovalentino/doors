/* Ordinary Windows x64 fixture. No Door headers, imports, or runtime shortcuts. */
#define WIN32_LEAN_AND_MEAN
#define WINVER 0x0a00
#define _WIN32_WINNT 0x0a00
#include <windows.h>

static const char message[] =
    "WINDOWS_X64_OK relocation=1 invalid_handle=1 invalid_buffer=1 last_error=1 abi=1 zero=1 selector=1\r\n";
/* This writable absolute pointer requires the PE loader to apply a DIR64 fixup. */
static const char *volatile relocated_message = message;

extern int VerifyAbi(int (*callback)(void));

static int windows_calls(void)
{
    DWORD count = 123;
    DWORD error;
    HANDLE output = GetStdHandle(STD_OUTPUT_HANDLE);
    if (output == NULL || output == INVALID_HANDLE_VALUE)
        ExitProcess(101);

    SetLastError(0x1234);
    if (GetLastError() != 0x1234)
        ExitProcess(102);
    {
        HANDLE invalid = GetStdHandle(0);
        error = GetLastError();
        if (invalid != INVALID_HANDLE_VALUE || error != ERROR_INVALID_HANDLE)
            ExitProcess(109);
    }

    SetLastError(0x2345);
    if (GetStdHandle(STD_OUTPUT_HANDLE) != output || GetLastError() != 0x2345)
        ExitProcess(110);
    if (!WriteFile(output, NULL, 0, &count, NULL) || count != 0 || GetLastError() != 0x2345)
        ExitProcess(111);

    if (WriteFile(INVALID_HANDLE_VALUE, message, 1, &count, NULL))
        ExitProcess(103);
    error = GetLastError();
    if (error != ERROR_INVALID_HANDLE || count != 0)
        ExitProcess(104);

    count = 123;
    if (WriteFile(output, (const void *)(ULONG_PTR)1, 1, &count, NULL))
        ExitProcess(105);
    error = GetLastError();
    if (error != ERROR_INVALID_USER_BUFFER)
        ExitProcess(0x10000 | error);
    if (count != 0)
        ExitProcess(106);

    if (relocated_message != message || relocated_message[0] != 'W')
        ExitProcess(107);
    SetLastError(0x3456);
    if (!WriteFile(output, relocated_message, (DWORD)(sizeof(message) - 1), &count, NULL)
        || count != sizeof(message) - 1 || GetLastError() != 0x3456)
        ExitProcess(108);
    return 1;
}

__declspec(noreturn) void entry(void)
{
    int result = VerifyAbi(windows_calls);
    if (result != 1)
        ExitProcess((UINT)result);
    ExitProcess(73);
}

# Independent Windows x64 fixture

This ordinary no-CRT C application uses installed Windows SDK declarations and
`kernel32.lib`. It imports only `GetStdHandle`, `WriteFile`, `GetLastError`,
`SetLastError` and `ExitProcess`. It has no Door declarations or conditional paths.
The same executable bytes must run on the declared Windows reference and in Door.

The fixture requires an absolute data-pointer relocation, verifies per-thread
last-error round trips, rejects an invalid handle and an invalid input buffer, then
writes the exact success line and exits with code73. Distinct exit codes101–108
identify a failed assertion. No CRT, DLL lifecycle, broad console, or general
application compatibility is implied.

Use the supported xtask Windows reference build; compilation arguments, input
identities, imported symbols, PE metadata and captured reference results belong in
retained run evidence. Do not patch the executable or add Door-only imports.

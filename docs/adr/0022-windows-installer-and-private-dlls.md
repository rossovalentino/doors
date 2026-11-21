# ADR 0022: Windows Installer and private DLLs in Windows processes

Status: adopted 8 November 2026 (P16 M8). Plan:
`docs/design/p16-plan-20261102.md`. Subject to the P16 critic review.

## Context

P16's INSTALL-A asks for MSI packages with their standard actions, custom
actions of the EXE, DLL and script types, rollback, repair, upgrade and
removal, a durable record of an interrupted installation, EXE installers
and Settings > Installed apps. Two parts of the platform stood in the way:

- A Windows process could only map the system DLLs of its startup closure
  or of `WINDOWS_PROVIDER_LOAD` (fixed bases, authenticated initfs
  images). A DLL custom action is a DLL the package carries; NSIS-style
  installers load plugin DLLs from %TEMP%; applications load their own.
- Dynamic memory (VirtualAlloc) is never executable (P03/P05 W^X), so a
  user-mode loader cannot map such a DLL itself.

Windows Installer runs as a service with the SYSTEM token; Doors has no
such service and keeps Init allocation-free (it does not parse packages).

## Decision

1. **Windows Installer is msi.dll and msiexec.exe.** The engine
   (`crates/msi`: database, plan, sequences, standard and custom actions,
   rollback journal, product records) runs in the process that asks:
   msiexec.exe or a program calling MsiInstallProduct. A per-machine
   installation without the administrator's token stops before the
   execute sequence (740); msiexec hands the request (and the answers given
   in the UI sequence) to an elevated copy of itself through
   `HKCU\Software\Microsoft\Installer\Handoff` and User Account Control
   (`DoorRequestElevation` with `elevation::PACKAGE`, so the prompt names
   the package). With `/qn` it answers 1925, as Windows does. Amended 11
   November 2026 (P16 critic): the hand-off key is writable by the user's
   unelevated programs, so the window server takes PACKAGE requests from
   msiexec.exe only and gives the elevated copy one argument, a consent
   token (SHA-256 over the package path, the package's own SHA-256 and the
   command line; the request carries a SHA-256 of the last two, so a long
   command line still fits one message); the elevated copy acts only on a
   hand-off with that
   digest, only on that package path, and installs the package bytes it
   read once and hashed. What an elevated installation creates is owned by
   Administrators, not by the user's account.
2. **Unsupported tables fail.** Standard actions whose tables the engine
   does not carry out fail the installation with 1603 and a log line when
   the package has rows for them, instead of reporting an installation
   they did not make. Amended 11 November 2026: the engine now carries out
   Class, ProgId and AppId (RegisterClassInfo, RegisterProgIdInfo), MIME,
   Font (the title from the font's name table when the row has none),
   TypeLib (oleaut32's RegisterTypeLib, as Windows Installer calls it),
   SelfReg (DllRegisterServer with COM initialized; a module of the other
   bitness through that msiexec.exe's /y or /z), Environment (`=`, `+`,
   `-`, `!`, `*`, `[~]` append and prepend, once, and WM_SETTINGCHANGE),
   IniFile and RemoveIniFile (the profile functions' matching; ANSI files
   byte for byte, UTF-16 files with their BOM), DuplicateFile, MoveFile
   (left in place on removal, as documented), IsolatedComponent (copies
   InstallFiles makes in the application's folder, with its `.local`) and
   PublishComponent (the base-85 darwin descriptor and AppData). Each
   registry row belongs to the action that writes and removes it
   (`RegistryGroup`); registrations a module or type library makes itself
   are journaled as calls rollback reverses. What remains unsupported
   (1603 when the package has rows): Patch (no .msp patching), the ODBC
   tables, MsiAssembly (.NET and Win32 side-by-side assemblies, with
   P18's activation contexts), Complus, SFPCatalog, MsiServiceConfig and
   MsiServiceConfigFailureActions, and the administrative installation.
   LockPermissions and MsiLockPermissionsEx rows are logged and not
   applied (storage keeps four ACL entries per object and registry keys
   have no descriptors, ADR 0024), and a ServiceInstall StartName other
   than LocalSystem is logged and the service runs as LocalSystem (ADR
   0026); the installation goes on, as their absence would not stop it
   working.
5. **Bitness.** (Amended 11 November 2026.) A 32-bit (Intel or empty
   platform) package installs in the 32-bit SysWOW64\msiexec.exe (a real
   i686 build) and a 64-bit one in System32's: msiexec.exe and the
   MsiInstallProduct family hand a package of the other bitness to that
   msiexec.exe with an equivalent command line and wait for it (a 32-bit
   WiX Burn engine installing an x64 package gets the x64 engine), so a
   package's DLL custom actions load at its bitness. Registry writes are
   per component, as Windows Installer makes them: a 32-bit component's
   (every component of a 32-bit package, and those of a 64-bit package
   without msidbComponentAttributes64bit) in the 32-bit view
   (KEY_WOW64_32KEY), a 64-bit component's in the 64-bit view; AppSearch's
   RegLocator follows msidbLocatorType64bit; a registry key path is `0n:`
   or `2n:`. The Installer's own records are in the 64-bit view and a
   32-bit product's Add/Remove Programs entry in the 32-bit one (both
   measured on Windows 11 26H2).
3. **Private DLLs.** `WINDOWS_PRIVATE_LOAD` (application roles only): the
   caller passes a DLL's bytes; the kernel captures them into a process
   lease (the executable capture arena), requires IMAGE_FILE_DLL, plans
   the DLL with the process's modules — after the system DLLs its imports
   need, at one of `PRIVATE_MODULES` (4) fixed slots from
   `PRIVATE_BASE`, relocated there by the same validated fixup plan as an
   executable — and maps it read/execute only through that plan. Every
   module already mapped keeps its index and base. ntdll's LoadLibrary
   uses it for any DLL that is not a system DLL, runs DLL_PROCESS_ATTACH
   and answers a second load of the same bytes with the mapped module.
   Private DLLs stay mapped for the process, as system DLLs loaded after
   startup do. Amended 11 November 2026: Doors' own system DLLs resolve
   from the module catalog first; any other bare name follows Windows'
   safe search order (`door_windows_abi::dll_search`): a KnownDLL only in
   System32; otherwise the application's folder, System32, the Windows
   folder, then the current directory, which is never searched for a name
   Windows ships in System32 (3,758 names hashed from Windows 11 26H2, so
   a planted copy of a Windows DLL in the current directory is not
   loaded). LOAD_LIBRARY_SEARCH_* restrict the search to the folders they
   name and never the current directory; ".dll" is appended to a name
   without an extension. DONT_RESOLVE_DLL_REFERENCES loads as LoadLibrary
   does (DllMain runs; the flag is deprecated). The data-file flags
   (AS_DATAFILE, AS_DATAFILE_EXCLUSIVE, AS_IMAGE_RESOURCE) answer a module
   already loaded by that bare name, or a Doors system DLL, with the
   module; otherwise they search the same way and lay the file (at most
   64 MiB) out as its image in a heap block, never executed, for the
   resource functions, with Windows' handle marking (bit 0 or 1);
   FreeLibrary releases it. LoadLibraryA, LoadLibraryExA and
   GetModuleHandleA convert through the ANSI code page.
4. **Durability.** Before each change the engine appends its undo record
   to the rollback script (`C:\Windows\Installer\Config.Msi\<n>.rbs`,
   flushed) named by `HKLM\…\Installer\InProgress`; the installing process
   holds the script unshared. A later session that finds the record and an
   unheld script (or an earlier boot) rolls the installation back first.
   Custom actions that change the machine outside the script are recorded
   (`nontransactional`); rollback runs paired rollback actions and logs
   what it cannot undo.

## Consequences

- DLL custom actions run in the installing process, not in a separate
  custom-action server; a crashing DLL takes the installation with it
  (the next session rolls it back).
- A process loads at most four private DLLs; a private DLL may import only
  system DLLs (not another private DLL).
- Script custom actions fail with 2738/2739 (no script engines).
- A 64-bit package's 32-bit DLL custom actions fail (1723, with the cause
  logged): Doors has no custom-action server of the other bitness.
- Fonts are registered as Windows registers them, but GDI renders with
  Doors' bundled faces only, so an installed font is not yet usable.
- Every registration writes into the Doors registry, whose capacity (128
  keys and 384 value records per volume, 48-unit names, 2 KiB values)
  bounds what a package may register; the registry hive's redesign is
  planned before P18. Within that, registry names follow Windows' rules
  (since 11 November 2026): a key name may hold any character but `\`
  (MIME types such as `application/x-doors-reg`), a value name any but
  NUL.
- The rollback folder is `C:\Windows\Installer\Config.Msi` (the drive's
  root is not writable on Doors).
- Registry names longer than the Doors registry's 48 units (SharedDLLs
  paths) are hashed; the Doors process limit (13) bounds how many
  programs an installation may run at once.

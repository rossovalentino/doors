# ADR 0026: The Service Control Manager runs in Init over the registry

Status: adopted 11 November 2026 (P16 M7). Plan and progress:
`docs/design/p16-plan-20261102.md` ("M7 service recovery, hung controls,
denial, Services page" and "M7 closed"). Subject to the P16 critic
review.

## Context

Windows programs and installers install and run services through
advapi32 (OpenSCManager, CreateService, StartService, ControlService,
Change/QueryServiceConfig(2), StartServiceCtrlDispatcher,
SetServiceStatus) and `sc.exe`; Windows Installer's ServiceInstall and
ServiceControl tables use the same calls. On Windows, services.exe owns
service state, reads configuration from
`HKLM\SYSTEM\CurrentControlSet\Services`, starts service processes,
waits for their dispatcher (ServicesPipeTimeout, 30 s), delivers
controls, applies failure actions and writes events 7011, 7031, 7034
and 7036. Doors already had Init, which starts and supervises its own
system services with restart and backoff (P08) and is the only process
allowed to start programs on another user's behalf.

## Decision

1. **Init hosts the manager** (`services/src/init/scm.rs`, protocol
   `door_service_protocol::scm`, operations 0x7000–0x700f). Its state is
   host-testable and allocation-free, as Init is: at most 16 services
   (Doors' own four included), four dependencies and four failure
   actions each.
2. **The registry holds configuration, the manager holds state.**
   advapi32 and `sc` write ImagePath, DisplayName, Start, Type,
   ObjectName, DependOnService, FailureActions (with its 20-byte
   header), FailureCommand and the rest where Windows keeps them, then
   ask the manager to RELOAD. The manager answers only what it alone
   knows: SERVICE_STATUS_PROCESS, the service's process, and pending
   controls, with Windows' error codes.
3. **Service processes speak to the manager through advapi32.**
   StartServiceCtrlDispatcher REGISTERs (the manager tells the process
   which service it runs; the table's name is ignored as on Windows),
   SetServiceStatus reports, the dispatcher fetches NEXT_CONTROL and
   reports CONTROL_DONE when the handler returns; ControlService waits
   for that, failing with ERROR_SERVICE_REQUEST_TIMEOUT and event 7011
   after 30 seconds.
4. **Rights follow the caller's token** (ADR 0024): starting,
   configuring and deleting need an elevated administrator; a standard
   user may list and query (SCM-D). Failure actions (restart, recovery program) run as
   the service does (item 6); advapi32 accepts a REBOOT action only when the
   caller's token has SeShutdownPrivilege enabled (a client-side check:
   Init sees the kernel's authority, not advapi32's privilege state).
5. **Dependencies start first** (amended 11 November 2026 after the P16
   critic): StartService starts the stopped services a service depends on,
   in dependency order, and launches each once what it depends on runs; a
   cycle is ERROR_CIRCULAR_DEPENDENCY (1059), a missing dependency
   ERROR_SERVICE_DEPENDENCY_DELETED (1075), a disabled or failed one
   ERROR_SERVICE_DEPENDENCY_FAIL (1068). ImagePath is a command line: its
   arguments reach the service's process.
6. **Services run as LocalSystem in a logon session of their own**
   (amended 11 November 2026, owner decision; it answers the P16 critic's
   "services run elevated as whoever signs in"). The kernel provisions
   Init a logon permit for the service account (`SERVICE_ACCOUNT`, uid
   18, S-1-5-18) beside the two local accounts' at boot and nobody else
   one; it cannot be renewed. At the first sign-in of the boot Init opens
   that account's session (an administrator's) and starts every service
   in it with the full token, whoever asked: GetUserName answers SYSTEM,
   USERPROFILE is C:\Windows\system32\config\systemprofile (created then,
   with its AppData folders) and USERNAME the computer's name with `$`,
   as on Windows. The manager and the services stay up when the user
   signs out; a service's programs (CreateProcess) run in the same
   session; Init's Event Log, Task Scheduler, COM and network/audio
   resolution admit the session's processes. A launch without a working
   folder grant gets the child's own from the kernel.

## Consequences

- One supervisor for Doors' services and Windows services: crash loops,
  restart delays and reset periods use Init's clock (10 ms ticks), and
  events go to Init's Event Log.
- Settings > Services and `sc query` show the same list.
- Limits: 16 services; each started service gets a process of its own
  (no shared svchost-style host); ObjectName is stored, but every
  service runs as LocalSystem (no LocalService, NetworkService or user
  accounts); services start at the first sign-in of the boot, not at
  boot; Doors has no separate non-interactive desktop yet, so a service's
  windows appear on the signed-in user's desktop (Windows keeps them in
  session 0), where they take the user's input like any window: the
  user's keyboard and pointer, and the on-screen keyboard's injected keys
  (which go to the focused window, as Windows' UIAccess keyboard's do;
  nothing is injected while User Account Control is open). There is no
  User Interface Privilege Isolation, so a service that shows a window
  must treat what it receives there as the user's; window messages do
  not cross processes, so the shatter attack Windows' session 0 answers
  has no Win32 message path here;
  no delayed auto-start or trigger-start; RebootMessage stored but not
  broadcast. Raising the service
  count is a capacity change and needs the owner's approval like the
  other kernel and Init limits.
- Guest evidence: p16-scm (SCM-A, SCM-D, and SCM-R under TCG for the
  reboot action).

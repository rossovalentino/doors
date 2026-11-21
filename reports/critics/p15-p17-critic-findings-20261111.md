# P15-P17 critic findings (11 November 2026)

Read-only critics, one per phase (owner decision of 2 November 2026). These
are the findings as reported; the fixes and their evidence are recorded in
the phase plans. Formal critic records bound to a qualifying verify report
come with each phase's gate.

## P15 (network, audio and media) — not accepted as reported

1. Blocker: IPv6 not firewalled; loopback/own-address packets from the wire
   accepted (firewall.rs ipv4() None for IPv6, 127/8 source passes, AnyIP
   127/8 and ::1 on the wire interface; is_own trusts wire sources).
2. Major: tiny-fragment bypass (first fragment < 14 TCP bytes passes; later
   fragments pass; smoltcp reassembles IPv4).
3. Major: rule model fails open (unknown keys ignored, RA4 keywords skipped).
4. Major: raw IP_HDRINCL: header destination/source not checked against the
   sendto address and firewall; any user may spoof.
5. Major: predictable randomness (smoltcp seed = MAC ^ tick; dnsapi
   transaction IDs fixed sequence; ephemeral ports sequential; DNS reply
   question not compared).
6. Major: audio playback device loss not implemented (present never
   cleared, poll ignores errors, capture never recovers); ledger overclaims.
7. Major: mixer drops one output frame at every section boundary inside a
   period (services/audio lib.rs `continue`).
8. Major: 16 connections per service system-wide; per-channel socket
   quota: one process can starve door.network / door.audio.
9. Major: evidence gaps: no HTTPS success or certificate-error steps in the
   guest; no hostile frames at door-network; firewall denial never shown
   (ping retries); capture revocation while recording; device loss,
   overruns, drift.
10. Minor: microphone policy allows all until the shell pushes it.
11. Minor: select caps sets at 64.
12. Minor: TLS IPv6 literal SAN; assess flags unrelated expired certs.
13. Minor: HTTP trailer lines and 1xx responses unbounded.
14. Minor: ADR 0021, P15-VPN, P15-CAPTURE tests, plan claims, mmdevapi x64
    lint missing from the gate.

## P17 (32-bit Windows applications) — not accepted as reported

1. Blocker: DS/ES/FS/GS not saved or reloaded on return to user mode; a
   null or 0x3B selector leaks to the next thread (cross-process DoS for x86).
2. Major: no 32-bit msvcrt.dll (PATHS32 lists it; 23 catalog slots).
3. Major: SysWOW64 holds no programs; CreateProcess("msiexec ...") from a
   32-bit process fails.
4. Major: PE characteristics check refuses obsolete flags Windows ignores
   (0x80, 0x8000, 0x10, 0x400, 0x800).
5. Major: TEB32 of non-main threads truncated at 0x800 (TlsSlots at 0xE10
   unmapped).
6. Major: section 68 gaps: cross-bitness COM activation, PEB32
   ProcessParameters in x64 layout, LoadLibraryEx datafile flags, untested
   scenarios.
7. Major: gate weaknesses: RtlUnwind/__try/__finally/C++ throw never run;
   no relocated private x86 DLL with DllMain/TLS callbacks; shared-key
   check vacuous.
8. Minor: unsigned pointer-width values sign-extended.
9. Minor: 2 GB invariant not enforced; large images collide with DLL slots.
10. Minor: exception vectors beyond 0/6/14 not delivered; CONTEXT32 SegFs 0;
    CSTAR #UD address.
11. Minor: SYSENTER MSRs not programmed.
12. Minor: records (requirements gaps, implementation_state, ADR wording,
    TEB32/PEB32 fields left zero).

## P16 (COM, services and installers) — not accepted as reported

1. Blocker: files an elevated program creates are owned by the user's uid;
   the owner may replace the ACL without CHANGE_ACL, so the same user's
   unelevated processes can rewrite elevated installs, C:\Windows\Installer
   caches and Config.Msi journals (Windows: Administrators owns them).
2. Blocker: msiexec elevation hand-off reads its command line from
   HKCU\Software\Microsoft\Installer\Handoff (writable unelevated); PACKAGE
   prompts show an unverified caller-supplied name.
3. Blocker: HKCR merges HKCU first for elevated callers; per-user
   LocalServer32 hijacks elevated activation.
4. Blocker: Init's COM relay authenticates nothing (endpoint squatting,
   cross-integrity CONNECT/ACTIVATE, predictable IPIDs,
   CoInitializeSecurity ignored, no per-process quotas, Joiner reserves
   4 MiB per announced call).
5. Major: services run elevated in the interactive session as whoever signs
   in (standard users too) and read that user's HKCU.
6. Major: runas returns success before the prompt is answered (no
   ERROR_CANCELLED, no hProcess); lpParameters/lpDirectory dropped.
7. Major: many MSI standard actions are no-ops reporting success
   (RegisterClassInfo, RegisterTypeLibraries, SelfReg, MoveFiles,
   DuplicateFiles, WriteEnvironmentStrings, WriteIniValues, RegisterFonts,
   PatchFiles...).
8. Major: SCM: StartService does not start dependencies (1068), no cycle
   check (1059), no dependency order at autostart, ImagePath arguments and
   StartService arguments dropped.
9. Minor: deny ACEs for Administrators miss unelevated administrators.
10. Minor: crafted inputs abort parsers (authenticode certificate-table
    offset slice, typelib params_at underflow, CFB cyclic directory chain).
11. Minor: ADR 0024/0026/0025/0022 statements and P16 ledger claims not
    borne out (full token sources, SeShutdownPrivilege enforced client-side,
    COMOOP-A guest-only, unverified package name, no rpcrt4/impersonation
    owner decision, P16-INSTALL "implemented").
12. Gate: no negative elevation-boundary tests; INSTALL-A lacks section
    141.8 items (COM registration, run the product, reboot while installed,
    cancel mid-change, sentinels/inventory); `require` accepts any order.

## Fixed so far (11 November 2026, not yet guest-verified; image 8 pending)

- P15 1-5, 7, 8, 10-13: frame gate drops martians (127/8, ::1, own source)
  and IPv6 fragments whatever the firewall state; IPv6 SYN and ICMPv6 echo
  go through the policy; RFC 1858 fragment rules; rule model fails closed
  (unknown/unmodelled conditions: allow never matches, block applies;
  fields kept verbatim), RA4/RA6 ranges, keywords (LocalSubnet,
  DefaultGateway, DNS, DHCP) and ICMP types; IP_HDRINCL destination and
  source checked; kernel-entropy seed, random ephemeral ports, random DNS
  IDs and question check; mixer boundary fix with a sample-exact test;
  playback device loss (streams end, winmm returns buffers and answers
  MMSYSERR_NODRIVER, status not present, re-probe each second) and capture
  re-probe; microphone closed until the shell's policy; per-process
  connection cap (6) with 2 slots kept for boot services; select beyond
  64; TLS IPv6 literals and date judged on the chain used; HTTP trailer
  and 1xx bounds; mmdevapi/firewall/wireguard/pac/taskschd/wintrust/
  authenticode added to the gate lists.
- P16 1-4: elevated creations owned by Administrators (no owner rights
  for the unelevated same user); msiexec hand-off bound to a consent token
  over package, package SHA-256 and command line (PACKAGE only for
  msiexec; elevated copy verifies token, package path and bytes, reads the
  package once); elevated COM lookups (CLSID/AppID/TypeLib/Interface) use
  HKLM only; Init's COM relay scoped by session+uid+elevation, OLE<pid>
  only by that pid, per-connection quotas, random OIDs, Joiner reserves
  nothing ahead. P16 7 (unsupported MSI tables fail 1603 with a log
  line), 8 (dependencies started first, deferred launch, 1059/1068/1075,
  ImagePath arguments), 9 (deny Administrators applies to deny-only), 10
  (authenticode offset, typelib underflow, CFB chain bound).
- P17 1 (DS/ES/FS reloaded on every return to user mode; SYSENTER MSRs
  cleared), 2 (32-bit msvcrt.dll, errno in TEB32), 3 (SysWOW64\msiexec.exe
  and X86-C `msiexec` by name, 1605), 4 (PE characteristics as Windows),
  5 (full-page TEB32, DeallocationStack, TlsSlots mirror at 0xE10).
- Also: p16-oleaut regression from the x86 oVft scaling (dispinterface
  members keep the library's value; syskind scaling per pointer width).

## State at the usage limit (11 November 2026, later)

- Image 8 build failed only on the 32-bit msvcrt link (fixed since: the
  `__CxxFrameHandler3` stub); not rebuilt yet.
- Also done after the list above: P17 PEB32 ProcessParameters32, 2 GB
  limit and large-executable placement, fault translation (#BP, #GP, #MF,
  #XM; CONTEXT32 SegFs 0x53), LocalServer32 activation across bitness,
  X86-A unwind and relocated-DLL steps (Windows 10/10), X86-C shared App
  Paths key (exit 127) and `msiexec` by name (1605); ADRs 0021-0026 amended.
- In progress: runas outcome (window server ELEVATION_STATUS and
  `Elevation` in service-client done; user32 `DoorElevationResult`, shell32
  `elevate` with lpParameters and SEE_MASK_NOCLOSEPROCESS, msiexec waiting
  for the elevated copy not yet written).
- Left: P15 guest evidence (HTTPS success and certificate errors, hostile
  frames, firewall denial, capture revoked while recording, device loss),
  P16 services running elevated as the session user (owner decision),
  compatibility settings page (owner decision on 48-unit value names),
  LoadLibraryEx data-file loads, requirements/plan updates for the fixes,
  image 8 build and full guest regression.

## Second read of the fixes (11 November 2026, evening)

The same read-only critics (one per phase) re-read the fixes above against
the code. What they raised, and what was done:

- P16 Blocker: services could not start under the new service account
  (the SCM refused a launch without a working folder; storage refused
  uid 18). Fixed: a launch without a folder grant gets the child's own
  from the kernel; storage serves the service account; the account's
  profile folders are created when its session opens.
- P16: the service session and service windows on the user's desktop,
  the on-screen keyboard's injected keys and the absence of UIPI are now
  stated in ADR 0026; SCM-A's service records its token's LocalSystem SID
  and elevation (checked); LockPermissions and a non-LocalSystem StartName
  are logged and documented (ADR 0022). Also closed: StartService's
  argument vector (and `sc start <name> <words>`) reaches ServiceMain,
  checked by SCM-A.
- P16 runas: the request's size is checked before the prompt; outcomes are
  kept per requester (eight) and dropped when a requester disconnects;
  empty arguments survive (each argument NUL-terminated); shell32 checks
  the program's size before allocating; lpDirectory is documented as not
  honoured (ADR 0024).
- P17: the data-file loader read e_lfanew + 4 unchecked (fixed, and files
  are capped at 64 MiB); LoadLibrary searched only the application's
  folder and the current directory, and missed a native image path's
  folder (now Windows' safe search order with KnownDLLs, System32, the
  Windows folder, LOAD_LIBRARY_SEARCH_* and ".dll" appended, the current
  directory never for Windows' own DLL names); DONT_RESOLVE_DLL_REFERENCES
  loads normally; X86-A reads a resource through a data-file handle
  (Windows 10/10). ADR 0023 records FS = 0x2B against CONTEXT32's 0x53;
  ADR 0022's search-order text was corrected.
- P15: dnsapi compared QTYPE/QCLASS case-insensitively (now the name
  only, with a test whose oracle is independent of the code); the hostile
  frame test gained IPv6 martian, IPv6 fragment and overlapping-fragment
  oracles. HTTPS gained guest evidence: CONNECT-A HTTP-A steps 16-19
  against xtask's TLS 1.3 server (`p15_tls`, with a host test driving
  Doors' own TLS client through all four cases), identical to Windows 11
  26H2, and WinHTTP's SECURE_FAILURE reasons now leave out the problems
  the request ignored, as Windows' do.
- Also found by the image 9 regression and fixed: win32-gui-f2 and f3
  copied fixtures from `/system/bin`, which Doorsh (the Terminal's default
  shell since P16) maps to C:\system\bin; the fixtures are also staged in
  System32 and copied from there. msiexec's elevation request carried
  the whole hand-off command line (with every property the dialogs set) and
  overran one message (240 bytes), so Install from File Explorer failed
  with 1925 before the prompt; the request now carries a SHA-256 binding
  of the package hash and the line, the elevated copy checks the line, and
  every elevation request is bounded by the message size.

- Found by the image 10-12 guest runs and fixed: registry key names used
  the file-name rules, refusing '/', ':', '*', '?' and the like, so every
  MIME registration (Content Type\application/...) failed with 1406; keys
  now forbid only '\' and value names only NUL, in the writer and the
  on-disk decoder (storage test across remounts). C:\Windows\Fonts did not
  exist (fonts failed to install with 1310). The kernel parsed private DLLs
  as PE32+ only (32-bit LoadLibrary of an application DLL failed); a
  launch without a working-folder grant was refused by the descriptor's
  validation (services could not start as the service account);
  IsWow64Process failed on a child that had exited (the machine now lives
  on the process object). Init opened a storage connection for every Task
  Scheduler status query and every scheduler tick (a query under load
  answered RPC_S_SERVER_UNAVAILABLE); SCM-A checked the service's identity
  after the probe deleted its key (the probe now checks it at start); the
  GUI DLL build bound (120 s) was too short for gdi32.
- Found by the first P17 gate attempt (12 November 2026, inherited P08/P09
  diagnostics on fresh volumes) and fixed: Init's synchronous LOGON
  deadlocked with the session service, which resolved storage through Init
  to read the account book on first use (the book now loads before the
  service reports READY); the P08 workload still asserted that session
  apps are denied kernel entropy (the kernel serves them from their own
  DRBG since P14/P15); Windows programs required the desktop's asset pack,
  which the diagnostic Windows modes do not load (now granted when
  present). The kernel and session service log a refused launch or logon.
Still open, recorded in project/NEXT_STEPS.md: capture revoked while
recording, firewall denial and device loss in the guest; INSTALL-A's
reboot while installed and inventory; the negative elevation-boundary
cases; services' non-interactive desktop.
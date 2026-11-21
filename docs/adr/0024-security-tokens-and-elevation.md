# ADR 0024: Security tokens and elevation from the kernel's authority

Status: adopted 11 November 2026 (P16 M5). Plan and progress:
`docs/design/p16-plan-20261102.md` ("M5 tokens, security functions,
accounts, Open with"). Subject to the P16 critic review.

## Context

Windows programs ask their token who they are and what they may do:
GetTokenInformation (user, groups in a fixed order, elevation type,
integrity label, linked token, privileges, default DACL, logon SID),
CheckTokenMembership, AccessCheck, impersonation, and the security
descriptors of files, keys and processes. Installers and administrator
tools branch on these answers. Doors already had a kernel-authenticated
identity per process (P08: user, session epoch, groups such as Users and
Services) and User Account Control in the window server (P16 M1: a
consent prompt on a secure desktop that Init trusts for
`EXECUTE_ELEVATED`). A Windows-style token object in the kernel, with
SIDs, privileges and ACL evaluation for every kernel object, would
duplicate that identity and the storage service's own access checks.

## Decision

1. **The kernel's authority is the token's source of truth.** A session
   application's authority carries the user's account and three group
   bits (`door_abi::platform::group`): USERS, ADMINISTRATORS (enabled:
   User Account Control approved this program) and
   ADMINISTRATOR_ACCOUNT (the account is an administrator, so its
   unelevated programs hold Administrators for deny only and have a
   linked full token). The session service's account book says which
   accounts are administrators (SESSION_CREATE's argument). The full token
   comes from User Account Control's approval: Init's hash-checked launch
   (`EXECUTE_ELEVATED`) and administrator Terminal tabs
   (`TERMINAL_ELEVATED`); after that it is inherited, as on Windows, by
   what an elevated process starts, and by the recovery programs and
   scheduled tasks Init runs for an elevated request. A program cannot
   raise its own authority. Amended 11 November 2026: services run as the
   service account (LocalSystem, uid 18, S-1-5-18) in that account's own
   logon session, whose permit only Init receives at boot (ADR 0026 item
   6); the account is an administrator and its programs get the full
   token.
2. **advapi32 builds Windows' token views from that authority.** The SIDs
   are Windows' (local accounts S-1-5-21-2026-26300-1-RID), the groups
   come in Windows' order with the integrity label first, the elevation
   type is Default/Limited/Full as Windows reports it for the three
   shapes (standard user, administrator unelevated, administrator
   elevated), and the linked token is an identification token of the
   other elevation. Per-handle state that Windows keeps in the token
   object (enabled privileges, impersonation level, duplicated
   impersonation tokens) lives in advapi32's token handles; the
   authority it starts from cannot be changed through them.
3. **Access is enforced where the object lives.** Storage keeps each
   file's ACL (four entries) and checks it; hives enforce registry
   access; the kernel checks process and thread rights. advapi32 maps
   these to Windows' security descriptors and ACEs and back
   (Get/SetNamedSecurityInfo, Get/SetFileSecurity, SDDL); AccessCheck
   evaluates caller-supplied descriptors with Windows' order of checks.
4. **Privileges are advapi32's view.** AdjustTokenPrivileges keeps the
   enabled state per token handle; a check that depends on it (a REBOOT
   failure action needs SeShutdownPrivilege enabled) is made by advapi32
   on the caller's side. What a service such as Init enforces is the
   kernel's authority: configuring a service, its failure actions
   included, needs an elevated administrator.
5. **Ownership follows elevation.** A file or folder an elevated program
   creates is owned by Administrators (storage's `ADMINISTRATORS_OWNER`),
   as Windows' elevated token's default owner, so the same user's
   unelevated programs get no owner's rights over it; a deny entry for
   Administrators applies to an unelevated administrator (deny-only), and
   an elevated program's COM lookups see machine registrations only.

## Consequences

- Elevation is a property of how a process was started, as on Windows;
  every Windows API answer about the token is derived, so the views
  cannot disagree with what the kernel and storage enforce.
- The probes (`secprobe.exe`, SEC-L/SEC-F/SEC-S) compare Doors' answers
  with Windows 11's for the limited and full administrator tokens and a
  standard user.
- Deviations recorded in the plan: storage keeps four ACL entries and no
  inheritance flags; registry keys have no descriptors of their own; a
  standard user's approved program runs as that user with the full token
  (Windows runs it as the approving administrator); there is no
  CreateRestrictedToken, LogonUser or per-thread token in the kernel
  (impersonation is advapi32's view for that thread). ShellExecuteEx
  "runas" (amended 11 November 2026) waits for the answer: ERROR_CANCELLED
  when the user declines, the new process's handle with
  SEE_MASK_NOCLOSEPROCESS, and lpParameters split as CommandLineToArgvW
  splits them (empty arguments kept); a request too large for Init's
  launch message is refused before the prompt opens. The elevated program
  starts in the caller's current directory: lpDirectory is not honoured.
  The display service keeps the outcome of up to eight requesters until
  each asks for it, and drops a requester's outcome when it disconnects.

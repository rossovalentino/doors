# Explorer and Windows path presentation

The complete Microsoft [known-folder inventory](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)
is the reference for Windows folder identifiers and default locations. The
[pathname contract](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
distinguishes drive-absolute, drive-relative, root-relative, UNC and device paths.
Creating similarly named directories does not implement these APIs, DLL discovery,
environment expansion, ACLs, registry state or executable compatibility.

| Explorer spelling | Actual authenticated storage | Current behavior |
| --- | --- | --- |
| `C:\` | `/compat` | Existing compatibility view, including Users alias |
| `C:\Windows\System32` | `/compat/Windows/System32` | Existing protected namespace; no Microsoft DLLs installed |
| `C:\Users` | `/users` | Existing users directory; no access grant from spelling |
| `C:\Users\1001` | `/users/1001` | Actual development profile, not a fabricated user name |
| Desktop, Documents, Downloads, Music, Pictures, Videos | Real child directories of the profile | Persist through the existing journaled Storage owner |
| `%USERPROFILE%` and declared `shell:` profile-folder shortcuts | Same authenticated profile | Explicit supported aliases, not a general environment interpreter |
| Program Files, ProgramData, Public, AppData, LocalAppData, Roaming, SysWOW64 | Not provisioned by this UI change | Require coordinated package/configuration/profile/architecture services and known-folder APIs |
| UNC, device/extended paths, alternate streams, other drives | Unsupported | Explicit failure; never silently rerouted locally |

Explorer resolves Windows components with the backend's declared ASCII case fold.
Every predecessor is enumerated before advancing, including before `..`; an absent
or denied directory cannot be canceled lexically to bypass a permission check.
Browsing does not migrate the owner disk, rename identities or create a second
filesystem. The address bar retains the entered text, current directory and history
on errors. Successful navigation publishes only after directory enumeration.

The bounded shared line editor supports caret insertion, Home/End, arrows,
Shift-selection, Delete/Backspace, Ctrl+A and word deletion. Mouse positioning,
visible selection/caret and horizontal scrolling use the address font's metrics.
Input remains the declared ASCII keyboard profile; clipboard/IME/Unicode rendering
and the generic Windows known-folder interfaces remain separate P12/P13 work.

The owner's2021 untabbed Explorer screenshot remains the visual baseline for
toolbar/address geometry. Microsoft [Explorer navigation](https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-explore-and-navigate-file-explorer-in-windows)
and [keyboard shortcuts](https://support.microsoft.com/en-au/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec)
guide address focus and breadcrumb actions. This is an original native browser,
not a complete or pixel-identical Windows shell.

# Settings unification: legacy Windows surfaces in Doors Settings (P14 WS-D)

Owner request (26 October 2026): the "advanced setting" pages added to Doors
Settings did not look like Windows 11 at all. Remake them in the visual
language of the other Settings pages. Where it is necessary and proper, add
pages and functionality so that every legacy mini-page goes away. Inventory
every Windows legacy page, window and control, and plan how to unify them
under Settings without overloading the user. Every legacy command and window
redirects to a Settings page, and the result has to be easy for an
inexperienced user to navigate.

This document holds the inventory, the plan (information architecture,
redirect table, order of work) and what was implemented. The redirect table
is generated from `frameworks/settings-map` (`door_settings_map::REDIRECTS`,
`SECTIONS` and `Page`), which the shell's Run box, Start search and launcher
use as well.

## Sources

* The owner's PC: Windows 11 Pro 26H2, build 10.0.26300. On 26 October 2026,
  a read-only probe launched each command, recorded the window that opened
  (its class, title and process), captured it with PrintWindow plus a UIA
  text dump, and closed it again. It sent no input and changed no settings.
  Results are in
  `reports/artifacts/reference-26h2/settings-unification/probe.tsv`, with PNG and
  `.uia.tsv` captures next to it. 48 `ms-settings:` pages were captured the
  same way.
* Commands that elevate (MMC snap-ins, `regedit`, `msconfig`,
  `SystemProperties*`) were not launched, by the coordinator's rule. For
  those, the 26H2 state comes from Microsoft documentation and from the
  links on the captured Settings pages. One `SystemPropertiesRemote` window
  auto-elevated during the probe and was left for the owner to close.
* Door before this work: `settings_catalog.rs` (the Windows-style pages) and
  the former `settings/advanced.rs`, `settings/advanced/reference.rs`,
  `settings_admin.rs` and `settings_reference_layout.rs`. Those four drew 16
  "Advanced" tool pages with their own fixed-coordinate tables, buttons and
  fields, under a non-Windows "Advanced" rail group. They have been removed.

Legend for "26H2": **S** means the surface opens Settings (observed redirect),
**L** means a legacy window (observed), and **L\*** means legacy per Microsoft
documentation (not launched because it elevates).

## Inventory

### Control Panel and its applets

| Surface (command) | 26H2 | Windows 26H2: in Settings / still legacy only | Door before P14 | Door target (page > section) |
| --- | --- | --- | --- | --- |
| Control Panel (`control`, category and All Items views) | L | Shell to every applet; most content duplicated in Settings | none | Home (Settings is the control panel) |
| System (`control system`, `/name Microsoft.System`) | S | Settings > System > About | System > About | System > About |
| System Properties (`sysdm.cpl`) | L | Computer Name, Hardware, Advanced, System Protection, Remote tabs (see below) | none | per tab, see System Properties |
| Power Options (`powercfg.cpl`) | L | Settings has Power mode, timeouts, lid/button; plans and advanced plan settings legacy | Power (unavailable rows) | System > Power > Power options |
| Sound (`mmsys.cpl`) | L | Settings > System > Sound has devices and volume; Sounds scheme, Communications, per-device advanced are legacy | Sound (no device) | System > Sound (All sound devices); unavailable until P15 audio |
| Network Connections (`ncpa.cpl`, `control netconnections`) | L | Settings > Network & internet > Advanced network settings lists adapters; adapter properties/TCP-IP legacy | Advanced network page (old style) | Network & internet > Advanced network settings > Network adapters; unavailable until P15 |
| Network and Sharing Center | L | Advanced sharing in Settings; the center itself legacy | none | Network & internet > Advanced network settings |
| Programs and Features (`appwiz.cpl`) | L | Settings > Apps > Installed apps; Windows Features and "view installed updates" legacy | Apps > Installed apps | Apps > Installed apps |
| Windows Features (`optionalfeatures`) | L | Settings > Apps > Optional features links "More Windows features" (legacy dialog) | none | Apps > Optional features (unavailable: no component manager) |
| User Accounts (`control userpasswords`) | L | Settings > Accounts > Your info / Other users; UAC slider and some options legacy | Accounts > Users & groups (old style) | Accounts > Your info |
| Advanced User Accounts (`netplwiz`, `userpasswords2`) | L\* | Auto sign-in, user groups legacy | none | Accounts > Other users (unavailable) |
| Credential Manager (`/name Microsoft.CredentialManager`, `keymgr.dll`) | L | Web and Windows credentials vault legacy | none | Accounts > Saved credentials (unavailable: no vault) |
| File Explorer Options (`control folders`) | L | 26H2 System > Advanced > File Explorer has extensions, hidden files, full path; the full dialog is legacy | System > Advanced (unavailable row) | System > Advanced > File Explorer |
| Fonts (`control fonts`, `Microsoft.Fonts`) | S | Settings > Personalization > Fonts (with a "redirected here" banner); `shell:Fonts` folder still in Explorer | Personalization (unavailable card) | Personalization > Fonts (new page) |
| Region (`intl.cpl`, `control international`) | L | Settings > Time & language > Language & region; custom formats and administrative (system locale) legacy | Language & region | Time & language > Language & region > Regional format |
| Date and Time (`timedate.cpl`) | L | Settings > Time & language > Date & time; additional clocks and internet time dialog legacy | Date & time | Time & language > Date & time |
| Mouse (`main.cpl`, `control mouse`) | L | Settings > Bluetooth & devices > Mouse; pointer schemes, double-click speed legacy | Bluetooth (unavailable card) | Bluetooth & devices > Mouse (unavailable: no pointer settings provider) |
| Keyboard (`control keyboard`) | L | Character repeat legacy (26H2 Settings lists Keyboard: "Character repeat, hotkeys") | none | Bluetooth & devices > Keyboard (unavailable) |
| Indexing Options | L | Settings > Privacy & security > Searching Windows links it | Search (unavailable) | Privacy & security > Search > Find files |
| Internet Options (`inetcpl.cpl`) | L | Proxy in Settings; everything else legacy | none | Network & internet > Advanced network settings > Internet options |
| Windows Defender Firewall (`firewall.cpl`, `wf.msc`) | L / L\* | Windows Security app > Firewall & network protection; rules legacy | none | Network & internet > Advanced network settings > Firewall (unavailable until P15) |
| Devices and Printers (`control printers`) | S | Settings > Bluetooth & devices > Printers & scanners | Bluetooth (unavailable) | Bluetooth & devices > Printers & scanners |
| Ease of Access Center (`access.cpl`) | L | Settings > Accessibility covers all of it | Accessibility | Accessibility |
| Windows Tools / Administrative Tools (`control admintools`) | L | Folder of MMC tools; nothing in Settings | Advanced rail group | System > Advanced > Administrative tools (collapsed) |
| Recovery (`Microsoft.Recovery`) | L | Settings > System > Recovery has reset/advanced startup; recovery drive and System Restore legacy | Backup & restore (old style) | System > Recovery |
| Security and Maintenance (`wscui.cpl`) | L | Windows Security app + Troubleshoot | none | Privacy & security > Doors Security |
| Backup and Restore (Windows 7), File History | L | Settings > Accounts > Windows backup (OneDrive); local backup legacy | none | System > Recovery > File backup (unavailable) |
| AutoPlay | L | Settings > Bluetooth & devices > AutoPlay | Bluetooth (unavailable) | Bluetooth & devices > AutoPlay |
| Color Management (`colorcpl`) | L | Settings > Display > Color profile (26H2) | none | System > Display > Color profile (unavailable) |
| Game Controllers (`joy.cpl`) | L | none | none | Bluetooth & devices > Devices > Game controllers (unavailable) |
| Pen and Touch (`tabletpc.cpl`) | L | Settings > Bluetooth & devices > Pen & Windows Ink | Bluetooth (unavailable) | Bluetooth & devices > Pen & Doors Ink |
| Phone and Modem (`telephon.cpl`) | L | Dialing location legacy | none | Privacy & security > Location (unavailable) |
| Speech Recognition (`sapi.cpl`) | L\* | Settings > Time & language > Speech | Time (unavailable) | Time & language > Speech |
| Storage Spaces, iSCSI Initiator, BitLocker | L | Storage Spaces partly in Settings (Advanced storage settings); others legacy | none | System > Storage > Advanced storage settings; Disks & volumes |
| Sync Center, Work Folders, RemoteApp | L | Legacy | none | Network & internet > Advanced network settings > Sharing; Accounts; System > Advanced > Remote Desktop |
| Default Programs (`Microsoft.DefaultPrograms`) | S | Settings > Apps > Default apps | Apps (unavailable) | Apps > Default apps (unavailable until WS-A's registration database) |
| Troubleshooting | S | Settings > System > Troubleshoot | System > Performance & diagnostics (old style) | System > Troubleshoot |
| Personalization, Display, Taskbar names | S | Settings | Settings | Settings (same pages) |

### System Properties tabs

| Tab (command) | 26H2 | Windows 26H2 | Door target |
| --- | --- | --- | --- |
| Computer Name (`sysdm.cpl`, `SystemPropertiesComputerName`) | L | Settings > About > Rename this PC; domain/workgroup join legacy | System > About (rename: unavailable) |
| Hardware (`sysdm.cpl,,1`, `SystemPropertiesHardware`) | L\* | Device Manager link, driver installation settings | Bluetooth & devices > Devices |
| Advanced: Performance (`SystemPropertiesPerformance`) | L\* | Visual effects partly in Accessibility > Visual effects; virtual memory, DEP legacy | Accessibility > Visual effects; DEP: Privacy & security > Doors Security |
| Advanced: User Profiles | L\* | Legacy | Accounts > Your info |
| Advanced: Startup and Recovery | L\* | Legacy (default OS, dump settings) | System > Advanced > System configuration; System > Recovery |
| Advanced: Environment Variables (`rundll32 sysdm.cpl,EditEnvironmentVariables`) | L | Legacy dialog (26H2 has no Settings page) | System > Advanced > Environment variables (new, real) |
| System Protection (`sysdm.cpl,,4`, `SystemPropertiesProtection`) | L\* | Settings > Recovery > Point-in-time restore links it | System > Recovery > Point-in-time restore (unavailable) |
| Remote (`sysdm.cpl,,5`, `SystemPropertiesRemote`) | L | Settings > System > Remote Desktop; Remote Assistance legacy | System > Advanced > Remote Desktop (unavailable) |

### MMC snap-ins and administrative tools

| Tool (command) | 26H2 | Windows 26H2 | Door before P14 | Door target |
| --- | --- | --- | --- | --- |
| Computer Management (`compmgmt.msc`) | L\* | Legacy console of the tools below | none | System > Advanced > Administrative tools |
| Device Manager (`devmgmt.msc`, `hdwwiz.cpl`) | L\* | Bluetooth & devices lists user devices; full tree, drivers and resources legacy | "Devices & drivers" (old style, art and empty tables) | Bluetooth & devices > Devices: every device Doors has, with connection, driver and status, from the kernel boot inventory, the disk owner and the window server |
| Disk Management (`diskmgmt.msc`) | L\* | Settings > Storage > Advanced storage settings > Disks & volumes (properties, format, VHD, Dev Drive) | "Disk management" (old style) | System > Storage > Disks & volumes: disk, volume and journal from DiskInfo/VolumeUsage; partition editing unavailable |
| Services (`services.msc`) | L\* | Task Manager > Services tab; console legacy | "Services" (old style) | System > Advanced > Services: Doors services from Init discovery, Refresh, Open Task Manager |
| Event Viewer (`eventvwr`) | L\* | Legacy | "Event viewer" (old style, unavailable) | System > Advanced > Event logs: Settings changes this session (real); system and app logs unavailable (no log service) |
| Task Scheduler (`taskschd.msc`, `control schedtasks`) | L (opened without elevation) | Legacy | "Task scheduler" (old style, empty) | System > Advanced > Scheduled tasks: "None", create unavailable (no scheduler service) |
| Local Users and Groups (`lusrmgr.msc`) | L\* (Pro) | Settings > Accounts > Other users for basics | "Users & groups" (old style) | Accounts > Your info: account type, user ID, groups, session, integrity |
| Local Security Policy (`secpol.msc`) | L\* (Pro) | Legacy | none | System > Advanced > Policies > Local security policy (not applicable) |
| Group Policy (`gpedit.msc`, `rsop.msc`, `gpresult`) | L\* (Pro) | Legacy; "managed by your organization" banners | "Group policy" (old style, empty) | System > Advanced > Policies: "None applied"; Group Policy is **not applicable** (no domain or policy store: every setting applies directly) |
| Performance Monitor (`perfmon`), Resource Monitor (`resmon`) | L\* | Task Manager > Performance | "Performance & diagnostics" (old style) | System > Troubleshoot > Performance (opens Task Manager, as Windows does); Run `resmon` opens Task Manager directly |
| Shared Folders (`fsmgmt.msc`), Print Management, Certificates (`certmgr.msc`) | L\* | Legacy | none | Network > Advanced sharing; Printers & scanners; Doors Security (unavailable) |
| Registry Editor (`regedit`, `regedt32`) | L\* | Legacy | "Registry" (old style 3-pane) | System > Advanced > Registry Editor: a guarded page with a warning InfoBar, current key, Go to a key / Up / Add a value, values and subkeys as cards, root key and view; edits in dialogs |
| System Configuration (`msconfig`) | L\* | Startup tab moved to Task Manager / Settings > Apps > Startup | "System configuration" (old style links) | System > Advanced > System configuration: each tab's Settings home |
| System Restore (`rstrui`), Recovery Drive | L | Settings > Recovery links | none | System > Recovery |
| Windows Memory Diagnostic, Troubleshooting (`msdt`) | L\* | Troubleshoot | none | System > Troubleshoot > System check |

### Other legacy windows

| Window (command) | 26H2 | Door target |
| --- | --- | --- |
| About Windows (`winver`) | L | System > About > Doors specifications |
| System Information (`msinfo32`) | L | System > About > Device specifications |
| DirectX Diagnostic Tool (`dxdiag`) | L\* | System > About > Device specifications |
| Windows Activation (`slui`) | L\* | System > Activation (not required on Doors) |
| Display scaling (`dpiscaling`) | S | System > Display > Scale |
| Bluetooth File Transfer (`fsquirt`) | L | Bluetooth & devices |
| Task Manager (`taskmgr`) | app | stays an app (Doors Task Manager) |

Totals: 63 legacy surfaces inventoried above (applets, tabs, snap-ins and
windows). They are reachable through 248 typed entry points: commands,
applets with tabs, canonical names, `control` keywords, snap-ins, shell
folders and 91 `ms-settings:` URIs. Every one resolves to one of 50 Settings
pages and, where needed, one of 66 sections.

## Plan

### Information architecture

* **Windows 11's twelve categories are the only rail entries.** The
  non-Windows "Advanced" rail group is gone. Every page sits under a
  category, at most two levels down (Category > Page > Subpage, the depth
  Windows uses for System > Storage > Disks & volumes). The breadcrumb
  heading shows every level, and each ancestor is a link. When the window
  is too narrow, whole crumbs drop from the left.
* **Legacy tools live where a Windows user would look**:
  * Devices: under Bluetooth & devices.
  * Disks & volumes: under Storage.
  * Network adapters and sharing: under Advanced network settings.
  * Recovery and Troubleshoot: under System.
  * Your info: under Accounts.
  * File system access: under Privacy & security.
  * Fonts: under Personalization.
  * Expert tools: System > Advanced (26H2's own home for developer and
    power options), inside a **collapsed "Administrative tools" section**.
    It covers Registry Editor, Services, Environment variables, Event logs,
    Scheduled tasks, Policies and System configuration. An inexperienced
    user sees one closed card with a plain description and a note that
    these tools are for experienced users.
* **Plain-language names**: Event Viewer is called "Event logs", Task
  Scheduler is "Scheduled tasks", Group Policy is "Policies", Performance
  Monitor is "Troubleshoot > Performance", Users & groups is "Your info",
  and Permissions & sharing is "File system". Every card has a description,
  and every unavailable control says why ("Unavailable: ...").
* **Search finds everything**:
  * Page names first (names starting with the query rank first).
  * Then section titles, then plain-language keywords.
  * Then legacy names and commands: "Device Manager", "sysdm.cpl",
    "Control Panel", "regedit", "Event Viewer".
  * A suggestion shows its destination with the name it was found by, for
    example "Devices · Device Manager".
* **Deep links** (Run, Start search, the launcher, `ms-settings:`) open the
  page. A section link also opens the collapsed section that holds it,
  scrolls to it and highlights it with an accent outline that fades after
  2.5 s, as Windows highlights a search target.
* **No dead controls**: a control either works (it persists, launches, reads
  a provider or opens a dialog) or is drawn disabled with its reason.
  Network and audio stay unavailable until P15.
* **Editing uses Windows ContentDialogs**: Registry values, keys and
  environment variables are edited in a centred dialog with a smoke layer.
  It has labelled text boxes, a Type combo, help text, an error line, and
  Save (accent) / Cancel. The dialog is modal: nothing behind it reacts,
  and Escape cancels.

### Pages (route table)

Page numbers are identities (guest scenarios check `SETTINGS PAGE id=N`, and
SETTINGS_NAVIGATE carries them). Numbers 0 to 48 never moved; Fonts (49) is
new. The table is generated from `door_settings_map::Page`.

| Id | Page | Keywords |
| --- | --- | --- |
| 0 | Home |  |
| 1 | System |  |
| 2 | Bluetooth & devices | mouse printers usb keyboard |
| 3 | Network & internet | wi-fi ethernet vpn airplane proxy |
| 4 | Personalization |  |
| 5 | Apps |  |
| 6 | Accounts | user sign-in |
| 7 | Time & language |  |
| 8 | Gaming |  |
| 9 | Accessibility |  |
| 10 | Privacy & security |  |
| 11 | Doors Update | windows update updates |
| 12 | System > About | version build processor specifications rename pc device name |
| 13 | System > Display | resolution scale orientation monitor screen brightness |
| 14 | Apps > Installed apps | programs uninstall applications |
| 15 | Personalization > Taskbar | search box alignment position small buttons |
| 16 | Personalization > Context menu | classic context menu right-click show more options file actions |
| 17 | System > Advanced > Registry Editor | registry keys values hkey |
| 18 | Bluetooth & devices > Devices | drivers hardware processor keyboard mouse disk display adapter |
| 19 | System > Storage > Disks & volumes | disk management partitions volumes drive format |
| 20 | Network & internet > Advanced network settings | network adapters sharing firewall internet options |
| 21 | System > Advanced > Services | background services running |
| 22 | System > Advanced > Environment variables | environment variables path |
| 23 | System > Advanced > System configuration | startup boot configuration |
| 24 | System > Advanced > Policies | policy group policy security policy |
| 25 | System > Advanced > Event logs | events log history activity |
| 26 | System > Advanced > Scheduled tasks | scheduled tasks schedule |
| 27 | Accounts > Your info | user account groups identity |
| 28 | Privacy & security > File system | permissions sharing access files folders |
| 29 | System > Troubleshoot | troubleshooter diagnostics checks problems |
| 30 | System > Power > Power options | power plans power options sleep hibernate |
| 31 | System > Recovery | reset restore backup recovery |
| 32 | System > Advanced > Developer tools | developer mode abi tools |
| 33 | System > Sound | volume audio speakers microphone |
| 34 | System > Notifications | do not disturb alerts |
| 35 | System > Power | sleep battery energy shut down |
| 36 | System > Storage | disk space drive capacity free space |
| 37 | Personalization > Background | wallpaper picture desktop background |
| 38 | Personalization > Colors | dark mode light mode accent transparency |
| 39 | Personalization > Themes | desktop icons theme |
| 40 | Personalization > Start | start menu size pinned recent all sections account |
| 41 | Time & language > Date & time | clock time zone date |
| 42 | Time & language > Language & region | language keyboard region format |
| 43 | Accessibility > Visual effects | animation motion transparency effects |
| 44 | System > Advanced | run dialog end task developer file explorer terminal |
| 45 | Privacy & security > Search | web results suggestions store search history |
| 46 | Accessibility > Contrast themes | high contrast aquatic desert dusk night sky low vision |
| 47 | Accessibility > Text size | font size larger text bigger text |
| 48 | Time & language > Typing | keyboard layout input method ime pinyin japanese korean add a keyboard |
| 49 | Personalization > Fonts | fonts typefaces install fonts |

### Order of work

1. Done: the inventory and the route table move into `frameworks/settings-map`
   (no_std, no dependencies), with `resolve`, `navigate_arg`,
   `decode_navigate` and `search`. WS-A's Run dialog already resolves
   through it.
2. Done: the catalog vocabulary gains working card buttons, InfoBars,
   run-time lists, fact values from real providers, registry choices,
   ContentDialogs, section lookup and shared scenario points.
3. Done: the 16 tool pages are rewritten as catalog pages, the old painter
   and layout modules are deleted, and the rail returns to twelve
   categories.
4. Done: providers are read on page entry, on deep link (the app update
   loop), on Refresh and on F5. Registry, environment-variable and
   access-check dialogs are added, along with the session event log and the
   section highlight.
5. Done: host tests; the guest runners `administration.rs` and
   `settings_reference.rs` now derive their points from the layout; the new
   `p14_settings.rs` runner.
6. Next, when other workstreams land:
   * Migrate persisted preferences to WS-B's typed settings registry.
   * Default apps: WS-A's registration database.
   * Sign-in options: password and PIN from WS-C's sign-in.
   * Network and audio pages: P15 drivers.

### Redirect table

Every legacy entry point, how it is typed, and the page and section it opens
(generated from `door_settings_map::REDIRECTS`). `resolve()` normalizes
input before lookup:
* It ignores case, quotes and paths, and drops `.exe`.
* `control /name X` resolves to X; `control x.cpl,,N` and `sysdm.cpl ,N` to
  the applet's tab N.
* `rundll32 shell32.dll,Control_RunDLL x.cpl` resolves to the applet;
  `mmc x.msc` and `explorer shell:...` to their targets.
* `ms-settings:` drops any `?query`. An unknown `ms-settings:` URI opens
  Home, as Windows does.

A command that is an app (`notepad`, `taskmgr`, `cmd`) is not a Settings
entry point: `resolve` returns `None` and the shell's own aliases launch the
app.

| Typed as | Legacy surface | Kind | Opens | Section |
| --- | --- | --- | --- | --- |
| `control` | Control Panel | Run command | Home |  |
| `shell:::{26ee0668-a00a-44d7-9371-beb064c98683}` | Control Panel | Shell folder | Home |  |
| `shell:::{21ec2020-3aea-1069-a2dd-08002b30309d}` | All Control Panel Items | Shell folder | Home |  |
| `shell:::{ed7ba470-8e54-465e-825c-99712043e01c}` | All Tasks | Shell folder | Home |  |
| `shell:controlpanelfolder` | Control Panel | Shell folder | Home |  |
| `control /name microsoft.system` | System | `control /name` | System > About |  |
| `control system` | System | `control` keyword | System > About |  |
| `shell:::{bb06c0e4-d293-4f75-8a90-cb05b6477eee}` | System | Shell folder | System > About |  |
| `sysdm.cpl` | System Properties | Control Panel applet | System > About |  |
| `sysdm.cpl,0` | System Properties: Computer Name | Control Panel applet | System > About |  |
| `systempropertiescomputername` | System Properties: Computer Name | Run command | System > About |  |
| `sysdm.cpl,1` | System Properties: Hardware | Control Panel applet | Bluetooth & devices > Devices |  |
| `systempropertieshardware` | System Properties: Hardware | Run command | Bluetooth & devices > Devices |  |
| `sysdm.cpl,3` | System Properties: Advanced | Control Panel applet | System > Advanced | Administrative tools |
| `systempropertiesadvanced` | Advanced system settings | Run command | System > Advanced | Administrative tools |
| `systempropertiesperformance` | Performance Options | Run command | Accessibility > Visual effects |  |
| `systempropertiesdataexecutionprevention` | Data Execution Prevention | Run command | Privacy & security | Doors Security |
| `sysdm.cpl,4` | System Properties: System Protection | Control Panel applet | System > Recovery | Point-in-time restore |
| `systempropertiesprotection` | System Protection | Run command | System > Recovery | Point-in-time restore |
| `sysdm.cpl,5` | System Properties: Remote | Control Panel applet | System > Advanced | Remote Desktop |
| `systempropertiesremote` | System Properties: Remote | Run command | System > Advanced | Remote Desktop |
| `sysdm.cpl,editenvironmentvariables` | Environment Variables | Run command | System > Advanced > Environment variables |  |
| `rundll32 sysdm.cpl,editenvironmentvariables` | Environment Variables | Run command | System > Advanced > Environment variables |  |
| `winver` | About Windows | Run command | System > About | Doors specifications |
| `msinfo32` | System Information | Run command | System > About | Device specifications |
| `dxdiag` | DirectX Diagnostic Tool | Run command | System > About | Device specifications |
| `slui` | Windows Activation | Run command | System | Activation |
| `desk.cpl` | Display | Control Panel applet | System > Display |  |
| `control desktop` | Personalization | `control` keyword | Personalization |  |
| `control color` | Color and Appearance | `control` keyword | Personalization > Colors |  |
| `control /name microsoft.display` | Display | `control /name` | System > Display |  |
| `control /name microsoft.personalization` | Personalization | `control /name` | Personalization |  |
| `control /name microsoft.taskbar` | Taskbar and Navigation | `control /name` | Personalization > Taskbar |  |
| `desk.cpl,0` | Desktop Icon Settings | Control Panel applet | Personalization > Themes | Desktop icon settings |
| `dpiscaling` | Display scaling | Run command | System > Display | Scale |
| `colorcpl` | Color Management | Run command | System > Display | Color profile |
| `control /name microsoft.colormanagement` | Color Management | `control /name` | System > Display | Color profile |
| `control fonts` | Fonts | `control` keyword | Personalization > Fonts |  |
| `control /name microsoft.fonts` | Fonts | `control /name` | Personalization > Fonts |  |
| `shell:fonts` | Fonts folder | Shell folder | Personalization > Fonts |  |
| `fontview` | Font Viewer | Run command | Personalization > Fonts | Install fonts |
| `mmsys.cpl` | Sound | Control Panel applet | System > Sound |  |
| `control /name microsoft.sound` | Sound | `control /name` | System > Sound |  |
| `powercfg.cpl` | Power Options | Control Panel applet | System > Power > Power options |  |
| `control /name microsoft.poweroptions` | Power Options | `control /name` | System > Power > Power options |  |
| `control /name microsoft.windowsmobilitycenter` | Windows Mobility Center | `control /name` | System > Power |  |
| `diskmgmt.msc` | Disk Management | MMC snap-in | System > Storage > Disks & volumes |  |
| `control /name microsoft.storagespaces` | Storage Spaces | `control /name` | System > Storage | Advanced storage settings |
| `cleanmgr` | Disk Cleanup | Run command | System > Storage |  |
| `dfrgui` | Optimize Drives | Run command | System > Storage | Advanced storage settings |
| `control /name microsoft.recovery` | Recovery | `control /name` | System > Recovery |  |
| `rstrui` | System Restore | Run command | System > Recovery | Point-in-time restore |
| `recoverydrive` | Recovery Media Creator | Run command | System > Recovery | Recovery drive |
| `control /name microsoft.backupandrestorecenter` | Backup and Restore (Windows 7) | `control /name` | System > Recovery | File backup |
| `sdclt` | Backup and Restore (Windows 7) | Run command | System > Recovery | File backup |
| `control /name microsoft.filehistory` | File History | `control /name` | System > Recovery | File backup |
| `control /name microsoft.bitlockerdriveencryption` | BitLocker Drive Encryption | `control /name` | System > Storage > Disks & volumes |  |
| `control /name microsoft.iscsiinitiator` | iSCSI Initiator | `control /name` | System > Storage > Disks & volumes |  |
| `iscsicpl` | iSCSI Initiator | Run command | System > Storage > Disks & volumes |  |
| `devmgmt.msc` | Device Manager | MMC snap-in | Bluetooth & devices > Devices |  |
| `hdwwiz.cpl` | Device Manager | Control Panel applet | Bluetooth & devices > Devices |  |
| `hdwwiz` | Add Hardware | Run command | Bluetooth & devices > Devices |  |
| `control /name microsoft.devicemanager` | Device Manager | `control /name` | Bluetooth & devices > Devices |  |
| `control printers` | Devices and Printers | `control` keyword | Bluetooth & devices | Printers & scanners |
| `control /name microsoft.devicesandprinters` | Devices and Printers | `control /name` | Bluetooth & devices | Printers & scanners |
| `shell:::{a8a91a66-3a7d-4424-8d24-04e180695c7a}` | Devices and Printers | Shell folder | Bluetooth & devices | Printers & scanners |
| `printui` | Printer settings | Run command | Bluetooth & devices | Printers & scanners |
| `printmanagement.msc` | Print Management | MMC snap-in | Bluetooth & devices | Printers & scanners |
| `main.cpl` | Mouse Properties | Control Panel applet | Bluetooth & devices | Mouse |
| `control mouse` | Mouse Properties | `control` keyword | Bluetooth & devices | Mouse |
| `control /name microsoft.mouse` | Mouse Properties | `control /name` | Bluetooth & devices | Mouse |
| `control keyboard` | Keyboard Properties | `control` keyword | Bluetooth & devices | Keyboard |
| `control /name microsoft.keyboard` | Keyboard Properties | `control /name` | Bluetooth & devices | Keyboard |
| `bthprops.cpl` | Bluetooth Devices | Control Panel applet | Bluetooth & devices |  |
| `control /name microsoft.bluetoothdevices` | Bluetooth Devices | `control /name` | Bluetooth & devices |  |
| `fsquirt` | Bluetooth File Transfer | Run command | Bluetooth & devices |  |
| `joy.cpl` | Game Controllers | Control Panel applet | Bluetooth & devices > Devices | Game controllers |
| `control /name microsoft.gamecontrollers` | Game Controllers | `control /name` | Bluetooth & devices > Devices | Game controllers |
| `tabletpc.cpl` | Pen and Touch | Control Panel applet | Bluetooth & devices | Pen & Doors Ink |
| `control /name microsoft.penandtouch` | Pen and Touch | `control /name` | Bluetooth & devices | Pen & Doors Ink |
| `control /name microsoft.autoplay` | AutoPlay | `control /name` | Bluetooth & devices | AutoPlay |
| `telephon.cpl` | Phone and Modem | Control Panel applet | Privacy & security | Location |
| `control /name microsoft.phoneandmodem` | Phone and Modem | `control /name` | Privacy & security | Location |
| `control /name microsoft.locationsettings` | Location Settings | `control /name` | Privacy & security | Location |
| `ncpa.cpl` | Network Connections | Control Panel applet | Network & internet > Advanced network settings | Network adapters |
| `control netconnections` | Network Connections | `control` keyword | Network & internet > Advanced network settings | Network adapters |
| `shell:::{7007acc7-3202-11d1-aad2-00805fc1270e}` | Network Connections | Shell folder | Network & internet > Advanced network settings | Network adapters |
| `control /name microsoft.networkandsharingcenter` | Network and Sharing Center | `control /name` | Network & internet > Advanced network settings |  |
| `inetcpl.cpl` | Internet Properties | Control Panel applet | Network & internet > Advanced network settings | Internet options |
| `control /name microsoft.internetoptions` | Internet Options | `control /name` | Network & internet > Advanced network settings | Internet options |
| `firewall.cpl` | Windows Defender Firewall | Control Panel applet | Network & internet > Advanced network settings | Firewall |
| `control /name microsoft.windowsfirewall` | Windows Defender Firewall | `control /name` | Network & internet > Advanced network settings | Firewall |
| `wf.msc` | Windows Defender Firewall with Advanced Security | MMC snap-in | Network & internet > Advanced network settings | Firewall |
| `control /name microsoft.remoteappanddesktopconnections` | RemoteApp and Desktop Connections | `control /name` | System > Advanced | Remote Desktop |
| `mstsc` | Remote Desktop Connection | Run command | System > Advanced | Remote Desktop |
| `fsmgmt.msc` | Shared Folders | MMC snap-in | Network & internet > Advanced network settings | Advanced sharing settings |
| `control /name microsoft.synccenter` | Sync Center | `control /name` | Network & internet > Advanced network settings | Advanced sharing settings |
| `control /name microsoft.workfolders` | Work Folders | `control /name` | Accounts |  |
| `appwiz.cpl` | Programs and Features | Control Panel applet | Apps > Installed apps |  |
| `control /name microsoft.programsandfeatures` | Programs and Features | `control /name` | Apps > Installed apps |  |
| `shell:::{7b81be6a-ce2b-4676-a29e-eb907a5126c5}` | Programs and Features | Shell folder | Apps > Installed apps |  |
| `shell:appsfolder` | Applications | Shell folder | Apps > Installed apps |  |
| `control /name microsoft.getprograms` | Get Programs | `control /name` | Apps > Installed apps |  |
| `optionalfeatures` | Windows Features | Run command | Apps | Optional features |
| `control /name microsoft.defaultprograms` | Default Programs | `control /name` | Apps | Default apps |
| `control userpasswords` | User Accounts | `control` keyword | Accounts > Your info |  |
| `control /name microsoft.useraccounts` | User Accounts | `control /name` | Accounts > Your info |  |
| `control userpasswords2` | Advanced User Accounts | `control` keyword | Accounts | Other users |
| `netplwiz` | Advanced User Accounts | Run command | Accounts | Other users |
| `lusrmgr.msc` | Local Users and Groups | MMC snap-in | Accounts > Your info |  |
| `control /name microsoft.credentialmanager` | Credential Manager | `control /name` | Accounts | Saved credentials |
| `control keymgr.dll` | Credential Manager | `control` keyword | Accounts | Saved credentials |
| `credwiz` | Stored User Names and Passwords | Run command | Accounts | Saved credentials |
| `certmgr.msc` | Certificates | MMC snap-in | Privacy & security | Doors Security |
| `wscui.cpl` | Security and Maintenance | Control Panel applet | Privacy & security | Doors Security |
| `control /name microsoft.actioncenter` | Security and Maintenance | `control /name` | Privacy & security | Doors Security |
| `control /name microsoft.windowsdefender` | Windows Defender | `control /name` | Privacy & security | Doors Security |
| `gpedit.msc` | Local Group Policy Editor | MMC snap-in | System > Advanced > Policies |  |
| `secpol.msc` | Local Security Policy | MMC snap-in | System > Advanced > Policies | Local security policy |
| `rsop.msc` | Resultant Set of Policy | MMC snap-in | System > Advanced > Policies |  |
| `gpresult` | Group Policy Results | Run command | System > Advanced > Policies |  |
| `intl.cpl` | Region | Control Panel applet | Time & language > Language & region | Regional format |
| `control international` | Region | `control` keyword | Time & language > Language & region | Regional format |
| `control /name microsoft.regionandlanguage` | Region | `control /name` | Time & language > Language & region |  |
| `timedate.cpl` | Date and Time | Control Panel applet | Time & language > Date & time |  |
| `control date/time` | Date and Time | `control` keyword | Time & language > Date & time |  |
| `control /name microsoft.dateandtime` | Date and Time | `control /name` | Time & language > Date & time |  |
| `sapi.cpl` | Speech Properties | Control Panel applet | Time & language | Speech |
| `control /name microsoft.speechrecognition` | Speech Recognition | `control /name` | Time & language | Speech |
| `control /name microsoft.texttospeech` | Text to Speech | `control /name` | Time & language | Speech |
| `lpksetup` | Install or uninstall display languages | Run command | Time & language > Language & region |  |
| `access.cpl` | Ease of Access Center | Control Panel applet | Accessibility |  |
| `control /name microsoft.easeofaccesscenter` | Ease of Access Center | `control /name` | Accessibility |  |
| `control folders` | File Explorer Options | `control` keyword | System > Advanced | File Explorer |
| `control /name microsoft.folderoptions` | File Explorer Options | `control /name` | System > Advanced | File Explorer |
| `control /name microsoft.indexingoptions` | Indexing Options | `control /name` | Privacy & security > Search | Find files |
| `control srchadmin.dll` | Indexing Options | `control` keyword | Privacy & security > Search | Find files |
| `control admintools` | Windows Tools | `control` keyword | System > Advanced | Administrative tools |
| `control /name microsoft.administrativetools` | Windows Tools | `control /name` | System > Advanced | Administrative tools |
| `shell:::{d20ea4e1-3957-11d2-a40b-0c5020524153}` | Windows Tools | Shell folder | System > Advanced | Administrative tools |
| `shell:common administrative tools` | Windows Tools | Shell folder | System > Advanced | Administrative tools |
| `compmgmt.msc` | Computer Management | MMC snap-in | System > Advanced | Administrative tools |
| `services.msc` | Services | MMC snap-in | System > Advanced > Services |  |
| `eventvwr` | Event Viewer | Run command | System > Advanced > Event logs |  |
| `eventvwr.msc` | Event Viewer | MMC snap-in | System > Advanced > Event logs |  |
| `taskschd.msc` | Task Scheduler | MMC snap-in | System > Advanced > Scheduled tasks |  |
| `control schedtasks` | Task Scheduler | `control` keyword | System > Advanced > Scheduled tasks |  |
| `perfmon` | Performance Monitor | Run command | System > Troubleshoot | Performance |
| `perfmon.msc` | Performance Monitor | MMC snap-in | System > Troubleshoot | Performance |
| `resmon` | Resource Monitor | Run command | System > Troubleshoot | Performance |
| `perfmon /res` | Resource Monitor | Run command | System > Troubleshoot | Performance |
| `mdsched` | Windows Memory Diagnostic | Run command | System > Troubleshoot | System check |
| `msdt` | Troubleshooting | Run command | System > Troubleshoot | System check |
| `control /name microsoft.troubleshooting` | Troubleshooting | `control /name` | System > Troubleshoot |  |
| `msconfig` | System Configuration | Run command | System > Advanced > System configuration |  |
| `regedit` | Registry Editor | Run command | System > Advanced > Registry Editor |  |
| `regedt32` | Registry Editor | Run command | System > Advanced > Registry Editor |  |
| `ms-settings:` | Settings | Settings URI | Home |  |
| `ms-settings:system` | System | Settings URI | System |  |
| `ms-settings:display` | Display | Settings URI | System > Display |  |
| `ms-settings:display-advanced` | Advanced display | Settings URI | System > Display | Advanced display |
| `ms-settings:nightlight` | Night light | Settings URI | System > Display | Night light |
| `ms-settings:sound` | Sound | Settings URI | System > Sound |  |
| `ms-settings:sound-devices` | All sound devices | Settings URI | System > Sound | All sound devices |
| `ms-settings:notifications` | Notifications | Settings URI | System > Notifications |  |
| `ms-settings:quiethours` | Focus | Settings URI | System > Notifications |  |
| `ms-settings:focus` | Focus | Settings URI | System > Notifications |  |
| `ms-settings:powersleep` | Power | Settings URI | System > Power |  |
| `ms-settings:batterysaver` | Energy saver | Settings URI | System > Power |  |
| `ms-settings:storagesense` | Storage | Settings URI | System > Storage |  |
| `ms-settings:disksandvolumes` | Disks & volumes | Settings URI | System > Storage > Disks & volumes |  |
| `ms-settings:savelocations` | Where new content is saved | Settings URI | System > Storage | Advanced storage settings |
| `ms-settings:multitasking` | Multitasking | Settings URI | System | Multitasking |
| `ms-settings:project` | Projecting to this PC | Settings URI | System | Projecting to this PC |
| `ms-settings:clipboard` | Clipboard | Settings URI | System | Clipboard |
| `ms-settings:remotedesktop` | Remote Desktop | Settings URI | System > Advanced | Remote Desktop |
| `ms-settings:activation` | Activation | Settings URI | System | Activation |
| `ms-settings:troubleshoot` | Troubleshoot | Settings URI | System > Troubleshoot |  |
| `ms-settings:recovery` | Recovery | Settings URI | System > Recovery |  |
| `ms-settings:about` | About | Settings URI | System > About |  |
| `ms-settings:advanced` | Advanced | Settings URI | System > Advanced |  |
| `ms-settings:developers` | For developers | Settings URI | System > Advanced | For developers |
| `ms-settings:bluetooth` | Bluetooth & devices | Settings URI | Bluetooth & devices |  |
| `ms-settings:devices` | Bluetooth & devices | Settings URI | Bluetooth & devices |  |
| `ms-settings:connecteddevices` | Devices | Settings URI | Bluetooth & devices > Devices |  |
| `ms-settings:printers` | Printers & scanners | Settings URI | Bluetooth & devices | Printers & scanners |
| `ms-settings:mousetouchpad` | Mouse | Settings URI | Bluetooth & devices | Mouse |
| `ms-settings:devices-touchpad` | Touchpad | Settings URI | Bluetooth & devices |  |
| `ms-settings:pen` | Pen & Doors Ink | Settings URI | Bluetooth & devices | Pen & Doors Ink |
| `ms-settings:autoplay` | AutoPlay | Settings URI | Bluetooth & devices | AutoPlay |
| `ms-settings:usb` | USB | Settings URI | Bluetooth & devices | USB |
| `ms-settings:network` | Network & internet | Settings URI | Network & internet |  |
| `ms-settings:network-status` | Network & internet | Settings URI | Network & internet |  |
| `ms-settings:network-ethernet` | Ethernet | Settings URI | Network & internet |  |
| `ms-settings:network-wifi` | Wi-Fi | Settings URI | Network & internet |  |
| `ms-settings:network-vpn` | VPN | Settings URI | Network & internet |  |
| `ms-settings:network-proxy` | Proxy | Settings URI | Network & internet | Proxy |
| `ms-settings:network-airplanemode` | Airplane mode | Settings URI | Network & internet |  |
| `ms-settings:network-mobilehotspot` | Mobile hotspot | Settings URI | Network & internet |  |
| `ms-settings:network-dialup` | Dial-up | Settings URI | Network & internet |  |
| `ms-settings:network-advancedsettings` | Advanced network settings | Settings URI | Network & internet > Advanced network settings |  |
| `ms-settings:personalization` | Personalization | Settings URI | Personalization |  |
| `ms-settings:personalization-background` | Background | Settings URI | Personalization > Background |  |
| `ms-settings:personalization-colors` | Colors | Settings URI | Personalization > Colors |  |
| `ms-settings:colors` | Colors | Settings URI | Personalization > Colors |  |
| `ms-settings:themes` | Themes | Settings URI | Personalization > Themes |  |
| `ms-settings:personalization-start` | Start | Settings URI | Personalization > Start |  |
| `ms-settings:taskbar` | Taskbar | Settings URI | Personalization > Taskbar |  |
| `ms-settings:fonts` | Fonts | Settings URI | Personalization > Fonts |  |
| `ms-settings:lockscreen` | Lock screen | Settings URI | Personalization |  |
| `ms-settings:appsfeatures` | Installed apps | Settings URI | Apps > Installed apps |  |
| `ms-settings:installed-apps` | Installed apps | Settings URI | Apps > Installed apps |  |
| `ms-settings:optionalfeatures` | Optional features | Settings URI | Apps | Optional features |
| `ms-settings:defaultapps` | Default apps | Settings URI | Apps | Default apps |
| `ms-settings:startupapps` | Startup | Settings URI | Apps | Startup |
| `ms-settings:appsforwebsites` | Apps for websites | Settings URI | Apps |  |
| `ms-settings:videoplayback` | Video playback | Settings URI | Apps |  |
| `ms-settings:yourinfo` | Your info | Settings URI | Accounts > Your info |  |
| `ms-settings:signinoptions` | Sign-in options | Settings URI | Accounts | Sign-in options |
| `ms-settings:otherusers` | Other users | Settings URI | Accounts | Other users |
| `ms-settings:emailandaccounts` | Email & accounts | Settings URI | Accounts |  |
| `ms-settings:workplace` | Access work or school | Settings URI | Accounts |  |
| `ms-settings:backup` | Doors backup | Settings URI | System > Recovery | File backup |
| `ms-settings:dateandtime` | Date & time | Settings URI | Time & language > Date & time |  |
| `ms-settings:regionformatting` | Language & region | Settings URI | Time & language > Language & region |  |
| `ms-settings:regionlanguage` | Language & region | Settings URI | Time & language > Language & region |  |
| `ms-settings:keyboard` | Typing | Settings URI | Time & language > Typing |  |
| `ms-settings:typing` | Typing | Settings URI | Time & language > Typing |  |
| `ms-settings:speech` | Speech | Settings URI | Time & language | Speech |
| `ms-settings:gaming-gamebar` | Game Bar | Settings URI | Gaming |  |
| `ms-settings:gaming-gamemode` | Game Mode | Settings URI | Gaming | Game Mode |
| `ms-settings:easeofaccess` | Accessibility | Settings URI | Accessibility |  |
| `ms-settings:easeofaccess-display` | Text size | Settings URI | Accessibility > Text size |  |
| `ms-settings:easeofaccess-visualeffects` | Visual effects | Settings URI | Accessibility > Visual effects |  |
| `ms-settings:easeofaccess-highcontrast` | Contrast themes | Settings URI | Accessibility > Contrast themes |  |
| `ms-settings:easeofaccess-mousepointer` | Mouse pointer and touch | Settings URI | Accessibility | Mouse pointer and touch |
| `ms-settings:easeofaccess-magnifier` | Magnifier | Settings URI | Accessibility | Magnifier |
| `ms-settings:easeofaccess-narrator` | Narrator | Settings URI | Accessibility | Narrator |
| `ms-settings:easeofaccess-closedcaptioning` | Captions | Settings URI | Accessibility | Captions |
| `ms-settings:easeofaccess-keyboard` | Keyboard | Settings URI | Accessibility | Keyboard |
| `ms-settings:privacy` | Privacy & security | Settings URI | Privacy & security |  |
| `ms-settings:privacy-location` | Location | Settings URI | Privacy & security | Location |
| `ms-settings:privacy-broadfilesystemaccess` | File system | Settings URI | Privacy & security > File system |  |
| `ms-settings:windowsdefender` | Doors Security | Settings URI | Privacy & security | Doors Security |
| `ms-settings:search-permissions` | Search permissions | Settings URI | Privacy & security > Search |  |
| `ms-settings:cortana-windowssearch` | Searching Windows | Settings URI | Privacy & security > Search | Find files |
| `ms-settings:windowsupdate` | Doors Update | Settings URI | Doors Update |  |
| `ms-settings:windowsupdate-history` | Update history | Settings URI | Doors Update | Update history |

### Sections (deep-link anchors)

A section names the visible title of a header, card, expander, hero or row
on its page. `every_section_anchor_exists_on_its_page` checks each one
against the real layout.

| Anchor | Page | Section |
| --- | --- | --- |
| `device-specifications` | System > About | Device specifications |
| `doors-specifications` | System > About | Doors specifications |
| `administrative-tools` | System > Advanced | Administrative tools |
| `end-task` | System > Advanced | End task |
| `file-explorer` | System > Advanced | File Explorer |
| `remote-desktop` | System > Advanced | Remote Desktop |
| `for-developers` | System > Advanced | For developers |
| `terminal` | System > Advanced | Terminal |
| `advanced-storage` | System > Storage | Advanced storage settings |
| `power-mode` | System > Power | Power mode |
| `screen-sleep` | System > Power | Screen and sleep |
| `power-button` | System > Power > Power options | Choose what the power buttons do |
| `reset-pc` | System > Recovery | Reset this PC |
| `advanced-startup` | System > Recovery | Advanced startup |
| `point-in-time-restore` | System > Recovery | Point-in-time restore |
| `file-backup` | System > Recovery | File backup |
| `recovery-drive` | System > Recovery | Recovery drive |
| `user-variables` | System > Advanced > Environment variables | User variables |
| `other-users` | Accounts | Other users |
| `sign-in-options` | Accounts | Sign-in options |
| `saved-credentials` | Accounts | Saved credentials |
| `printers` | Bluetooth & devices | Printers & scanners |
| `mouse` | Bluetooth & devices | Mouse |
| `keyboard` | Bluetooth & devices | Keyboard |
| `autoplay` | Bluetooth & devices | AutoPlay |
| `usb` | Bluetooth & devices | USB |
| `pen` | Bluetooth & devices | Pen & Doors Ink |
| `game-controllers` | Bluetooth & devices > Devices | Game controllers |
| `proxy` | Network & internet | Proxy |
| `network-adapters` | Network & internet > Advanced network settings | Network adapters |
| `sharing` | Network & internet > Advanced network settings | Advanced sharing settings |
| `internet-options` | Network & internet > Advanced network settings | Internet options |
| `firewall` | Network & internet > Advanced network settings | Firewall |
| `doors-security` | Privacy & security | Doors Security |
| `location` | Privacy & security | Location |
| `find-files` | Privacy & security > Search | Find files |
| `desktop-icons` | Personalization > Themes | Desktop icon settings |
| `color-profile` | System > Display | Color profile |
| `night-light` | System > Display | Night light |
| `scale` | System > Display | Scale |
| `advanced-display` | System > Display | Advanced display |
| `optional-features` | Apps | Optional features |
| `default-apps` | Apps | Default apps |
| `startup` | Apps | Startup |
| `regional-format` | Time & language > Language & region | Regional format |
| `time-zone` | Time & language > Date & time | Time zone |
| `speech` | Time & language | Speech |
| `system-check` | System > Troubleshoot | System check |
| `performance` | System > Troubleshoot | Performance |
| `activation` | System | Activation |
| `projecting` | System | Projecting to this PC |
| `clipboard` | System | Clipboard |
| `multitasking` | System | Multitasking |
| `sound-devices` | System > Sound | All sound devices |
| `game-mode` | Gaming | Game Mode |
| `transparency` | Personalization > Colors | Transparency effects |
| `visual-performance` | Accessibility > Visual effects | Animation effects |
| `mouse-pointer` | Accessibility | Mouse pointer and touch |
| `magnifier` | Accessibility | Magnifier |
| `narrator` | Accessibility | Narrator |
| `captions` | Accessibility | Captions |
| `keyboard-accessibility` | Accessibility | Keyboard |
| `install-fonts` | Personalization > Fonts | Install fonts |
| `local-security-policy` | System > Advanced > Policies | Local security policy |
| `startup-selection` | System > Advanced > System configuration | Startup selection |
| `update-history` | Doors Update | Update history |

## Implementation (26 October 2026)

### Visual language

The remade pages use only the existing Settings vocabulary (`settings_catalog`
blocks painted by `settings/pages.rs` with `settings/theme.rs`):
* Setting cards with Segoe Fluent symbols, a title over a description, and
  toggles, combos, values and chevrons.
* Expanders with 60 px child rows, section headers and notes.
* The breadcrumb heading.
* Light, dark and contrast themes, text-size growth, keyboard focus rings
  and hover/pressed states.

Every page, old and new, follows the 26H2 measurements
(`docs/design/fidelity-26h2-diff.md` G2–G6 and the owner's capture of
27 October 2026), with sizes from `door_design_tokens` where a token exists:
* **Title bar**: back, "Settings", then the "Find a setting" pill centred in
  the title bar (the trusted client-caption control: at most 516 px, 190 px
  from both window edges, fully rounded, accent underline when focused),
  then the caption buttons. Search is never in the rail; a window too narrow
  for a 120 px pill has no search box and Ctrl+F does nothing.
* **Rail**: the account block (60 px avatar at x 19, name at x 91), then the
  twelve categories from y 144: items `size.nav_item_height` (36) tall at a
  40 px pitch, inset 17 and 279 wide, a 3×14 accent pill (`radius.bar`), and
  a scroll indicator only while the rail is hovered.
* **Cards**: 69 px at a 73 px pitch (gap `space.xs`), symbol at 19, text at
  59, controls ending 17 px from the right, chevron centre 25 px from it.
  Group rows (Home > Recommended settings) are card height. A section header
  sits 27 px above its first card and 31 px below the cards before it.
* **About**: the device tiles (200×142 at a 212 pitch), the name card, the
  Device and Doors specifications (rows at a 25 px pitch, values at +174) and
  the related-links strip (System protection and Advanced system settings
  open their pages; Domain or workgroup is plain text because Doors has no
  domain; a link that does not fit is not drawn).
* **Date & time**: the clock header replaces the "Current time" card.
* **Home**: the device card and status tiles, Recommended settings (Storage,
  Installed apps, Taskbar: Doors keeps no usage history, and has no lock
  screen settings), Bluetooth devices and Personalize your device.

Four additions follow WinUI:
* **Card buttons**: a working WinUI Button on a card. A command that can't
  act now (Up at a root key) is drawn disabled.
* **InfoBar**: informational, with the accent info symbol and one or two
  lines.
* **Run-time lists**: one card per entry, with a chevron to open it or an
  Edit button.
* **ContentDialog**: smoke layer, 8 px card, 20 px semibold title, labelled
  32 px text boxes with an accent bottom edge when focused, a Type combo,
  help and error lines, and a footer of equal-width buttons with the
  primary one in the accent colour.

There are no new styles, assets or fonts.

### Real data behind each page

| Page | Provider |
| --- | --- |
| Devices | Kernel boot inventory (`hardware_info`: processors, ACPI power profile, chassis); window-server output and scale; Storage `disk_info`; current keyboard layout |
| Disks & volumes, Recovery | Storage `disk_info` (sectors, GPT volume and journal extents, write and flush) and `volume_usage` |
| Services | Init endpoint discovery for Storage, Session, Console, Window (Refresh / F5); Task Manager launch |
| Environment variables | HKEY_CURRENT_USER\Environment (registry, REG_SZ, created by the first save; checked writes); the launch environment read-only |
| Registry Editor | Registry snapshot/version-checked save (`registry_browser`), typed values (`registry_value`), explicit create |
| Event logs | Settings changes made in this session (toggles, combos, themes, text size, contrast, keyboards, registry/variable saves, checks) |
| Your info | The kernel identity Settings started with (UID, groups, session, epoch, integrity) |
| File system | Storage `metadata` under the session's permissions (Check access dialog) |
| Troubleshoot | Native ABI query, monotonic clock, Storage round trip; Task Manager launch |
| Developer tools | ABI version, timer rate; Terminal launch |

### Not applicable or unavailable, with reasons

* **Group Policy and Local Security Policy** are not applicable. Doors has
  no domain and no policy store, so every setting applies directly. The
  Policies page says so ("Applied policies: None").
* **Scheduled tasks**: there is no scheduler service, so the page shows
  "None" and Create is disabled.
* **Network adapters, sharing, firewall, internet options, audio**: no
  drivers until P15.
* **Partition editing, VHD, driver update, service start/stop, registry
  delete/rename, system variables, deleting variables**: the providers
  don't offer these operations, so each says so on its card or in a note.
* **System and application event logs**: Doors writes system events to the
  debug console only.
* **Default apps**: waits for WS-A's registration database.
* **Sign-in options**: waits for WS-C's credential sign-in (P14).
* **Fonts**: lists the font families the image provides by role.
  Installing fonts is unavailable.
* The registry backend holds 24 keys and 24 values per volume. A full
  registry is reported as "The registry is full".

### Tests

* `cargo test -p door-settings-map`, 5 tests. They cover every redirect key
  (reachable and resolving to its own row), every section, 96 inventory
  spellings, `navigate_arg` round trips with foreign sections rejected, and
  search ranking.
* `cargo test -p door-workspace`: all suites pass. `settings_reference_painter`
  has 35 tests, including:
  * the remade pages in both themes at 7 widths;
  * the breadcrumb links;
  * the registry dialog points and keyboard paths;
  * the 16 administration searches and history;
  * legacy-name search with section highlight;
  * deep links from legacy commands;
  * every section anchor present;
  * no dead buttons;
  * the lifecycle scenario's Settings points (rail rows and pills, card
    links, Display values after End at both snaps, title-bar search, Installed
    apps menus, theme tiles, heading background), all taken from
    `settings_catalog::points`, the same functions `lifecycle.rs` calls.
  Library tests pin the 26H2 metrics: the title-bar search (centred, 190 px
  margins, never in the rail), rail geometry, card and section rhythm, and
  the About and Date & time layouts.
* `cargo test -p door-xtask`:
  * `administration::registry_points_are_derived_from_the_settings_layout`
  * `p14_settings::{run_commands_and_searches_resolve_as_the_scenario_expects, navigation_points_are_derived_from_the_settings_layout}`
* Clippy with `-D warnings`: `door-workspace` (`--features native --target
  x86_64-unknown-none`, and host all-targets) and `door-settings-map` (host
  and `x86_64-unknown-none`).

### Needs the guest

* `desktop-advanced-settings` and the Settings steps of the wide lifecycle
  (`desktop-settings-layouts`, `desktop-settings-narrow-values`,
  `desktop-settings-themes`), `settings-reference` and the new
  `p14-settings` scenario. Their points are derived and host-pinned, but
  no QEMU run has been made (QEMU was on hold during this work).
* The window server now receives a 190 px side margin for Settings' caption
  control (`runtime.rs`, mirrored by `window_points.rs`); the guest should
  confirm the pill takes input and the title bar around it still drags.
* Visual comparison against the 26H2 captures in
  `reports/artifacts/reference-26h2/settings-unification/` is still to be done.

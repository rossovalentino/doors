# Settings parity with Windows 11 26H2

Status: plan, 12 November 2026. Owner request the same day: every Doors
Settings page, its behaviour and animation, identical to the owner's Windows
11 26H2 Settings; every built-in theme and wallpaper (the Windows dark theme
uses the dark Bloom); the Mica tint the wallpaper gives Settings; every
missing setting added; every legacy Control Panel page unified under Settings
(continuing `settings-unification.md`).

Left out on purpose (owner, same day):

- the help parts of every page: "Related support", "Get help", "Give
  feedback";
- telemetry and diagnostics: Diagnostics & feedback, activity history,
  tailored experiences, advertising ID, recommendations and offers;
- Microsoft's built-in AI (Copilot, Recall, Click to Do, AI components,
  agent tools, text and image generation, background AI tasks). In their
  place one page where the user connects an AI of their own: a model on
  their own machine or server, or a provider's API with their key;
- Microsoft promotions: Microsoft account upsell, Store suggestions,
  OneDrive, Phone Link and Game Pass prompts.

A setting Doors cannot provide yet stays on its page, disabled, with
"Currently unavailable" in its description, so nobody looks for it in
vain.

## NoneDrive (in place of OneDrive)

Doors' own sync folder, kept in step with a server the user owns: a home
NAS, Nextcloud, ownCloud, Synology, or any WebDAV server (WebDAV is what
all of them speak). Settings > Accounts > NoneDrive walks through setup:
server address (`https://`), user name and an app password (kept in the
credential vault, never in the profile), the server's certificate (shown
for confirmation and pinned when it is self-signed, as home servers
usually are), the local folder (`%USERPROFILE%\NoneDrive`), and which
folders to sync. Optionally through a Doors VPN connection when the server
is not reachable from outside. Files shows NoneDrive under Home with sync
state on each item (synced, syncing, conflict, error); conflicts keep both
copies, as OneDrive does. The existing feature switch (System > Advanced)
and the Files folder stay; the sync engine runs in the user's session
and stops with it.

Order: after the P17 gate closes P15 to P17 (the gate runs on the code frozen
before this work), before P18 M0a. Changing Settings during the gate would
invalidate its Settings scenarios.

Implementation (13 November 2026, in progress):

- `crates/webdav` (`door-webdav`, sans-IO, host-tested): request heads
  over `door-http` (PROPFIND Depth 1, GET, PUT with If-Match, MKCOL,
  DELETE, MOVE with Overwrite F, Basic authentication over TLS only), the
  207 multistatus parser (any `DAV:` prefix, CDATA, entities, per-propstat
  status), hrefs percent-encoded per segment and decoded relative to the
  sync folder, the index file, and the three-way planner (local now,
  server now, index): one side changed goes to the other, both changed
  keep both (`name-DEVICE.ext`, numbered if taken), a deletion follows
  only when the other side is unchanged, a lost index never deletes and
  compares same-size files byte for byte, a folder deleted on one side
  comes back when the other side changed something in it.
- Names Windows cannot hold (`<>:"\|?*`, trailing dot or space, CON,
  LPT1...), names equal but for case, and a file facing a folder are left
  alone and listed in the NoneDrive page. Server files over the volume's
  file limit (4,243,456 bytes until P18 M0a's ext2 at scale) are skipped
  and listed ("Too large for this PC for now"), never taken for deleted.
- Self-signed home servers: door-tls reports the leaf certificate's
  SHA-256 (`Client::leaf_sha256`); setup shows it and pins it after the
  user confirms; a different certificate later stops sync and asks again.
- The engine is a windowless session host like Start search's (Workspace
  manifest, its own mode and image), started by Init at sign-in when
  NoneDrive is set up and stopped with the session; it lists both sides
  every 30 seconds and when the sync folder changes, and reports its state
  to the page and to Files' item overlays.

## Your AI (in place of Windows' AI components)

Doors has no built-in AI. System > AI is where the user connects one of
their own, for apps that support it: a model on this PC or a home server
(any OpenAI-compatible endpoint: Ollama, LM Studio, llama.cpp's server,
vLLM) or a provider's API with the user's key. The page keeps the address,
the model name and the key (in the user's profile, like NoneDrive's app
password), and "Test" lists the endpoint's models (`GET /v1/models`) to
show it answers. Apps read the address and model from
`HKCU\Software\Doors\AI`; nothing in Doors calls the model by itself, and
nothing is sent anywhere until an app the user runs asks.

## References (captured 12 November 2026, owner approved)

- `out/acquisitions/settings-26h2/{dark,light}/<page>/`: every `ms-settings:`
  page the Settings app's own binaries name (266 links, 244 pages after
  removing links that start actions, enrollments, provisioning or dialogs),
  plus the three category pages without a link (Apps, Time & language,
  Gaming, reached through the navigation pane). For each page: screenshots
  at every scroll position (`plain-NN.png`), the same after expanding every
  collapsed card (`expanded-NN.png`), and the UI Automation tree of each
  (`*.uia.tsv`: control type, class, name, rectangle, automation id,
  toggle/value/selection/expand/range state). The owner's display: 2560x1600
  at 150 %, Settings maximized. Dark pass in the owner's mode; light pass
  with Color mode switched to Light and restored afterwards. These files are
  private (account name, e-mail, device and network names) and stay out of
  the repository.
- Outlines (`outline-{dark,light}/<page>.outline.txt`, generated): the
  page's breadcrumb and its content in order (groups, cards, toggles, combos
  with the selected item, sliders, check boxes, buttons, links) with
  Windows' automation ids. 160 distinct pages, matching the 160 pages of the
  Settings search index (`C:\Windows\ImmersiveControlPanel\Settings\
  AllSystemSettings_*.xml`, 1,750 searchable settings).
- Themes: `C:\Windows\Resources\Themes\{aero,dark,spotlight,themeA..D}.theme`
  and `C:\Windows\Web\Wallpaper\**` (22 JPEGs, 26 MB) plus the lock screen
  images in `C:\Windows\Web\Screen`.

## Themes and wallpapers

| Theme (26H2 name) | File | Wallpaper | Mode | Accent |
| --- | --- | --- | --- | --- |
| Windows (light) | aero.theme | Windows\img0 (Bloom) | Light | 0078D4 |
| Windows (dark) | dark.theme | Windows\img19 (dark Bloom) | Dark | 0078D4 |
| Windows spotlight | spotlight.theme | Spotlight\img50 | Light | 0078D4 |
| Glow | themeA.theme | ThemeA img20-23 slideshow, 30 min | Dark | 680081 |
| Captured Motion | themeB.theme | ThemeB img24-27 slideshow, 30 min | Dark | D0000C |
| Sunrise | themeC.theme | ThemeC img28-31 slideshow, 30 min | Light | 459BAC |
| Flow | themeD.theme | ThemeD img32-35 slideshow, 30 min | Light | 506773 |

Applying a theme sets wallpaper (or slideshow), app and system mode and
accent together, as Windows does; changing one of them afterwards makes the
current theme "Custom". So choosing Doors (dark) on Home or Themes gives
the dark Bloom, and choosing Dark under Colors alone keeps the wallpaper.
Doors shows the first two as "Doors (light)" and "Doors (dark)" (branding).

Themes page (measured from the 150% capture, logical px): a 250x156
preview with an 8 px black bezel, a translucent sample window and no
taskbar; Background, Color, Sounds and Mouse cursor entries in two columns
208 px apart from x 274 (icon at +10, text at +44); Save 130x32 at (282,
109), disabled until custom themes can be saved; then the "Current theme"
expander (value: the applied theme or Custom) holding one row of tiles 60 px
in, 120 px apart, 24 px below the header and 26 px above the edge. The
Store row is omitted. Desktop icon settings stays an in-page expander
(the classic dialog is unified into Settings).

Background page: the same bezel preview (it shows the solid color in Solid
color mode), then "Personalize your background" whose rows follow the kind,
as Windows swaps them:

- Picture: Recent images (five 100 px squares 4 px apart, 58 px in, no
  current-image ring), Choose a photo (Browse photos, currently
  unavailable: no photo picker), Choose a fit.
- Solid color: "Choose your background color" swatches, eight to a row,
  and Custom colors (View colors, currently unavailable). The 26H2 swatch
  values are not in the Settings binaries and could not be captured
  without changing the owner's background, so the swatches are the
  Desktop Background control panel's own palette (themecpl.dll, 33
  colors, black first), which also unifies that legacy page. Black is
  stored as 0x010101 because the preference's colour 0 means the default
  0x0063B1.
- Slideshow: the album (the current background's theme; Browse currently
  unavailable), Change picture every, Shuffle, "Let slideshow run even if
  I'm on battery power" (currently unavailable, always runs), Choose a fit.

Related settings: Contrast themes only, as on Windows.

Personalization landing: the same bezel preview 34 px below the hero top,
"Select a theme to apply" 300 px in, and the 3x2 theme grid 32 px below it
at a 120 x 98 px pitch; Windows' card symbols (Background E91B, Colors
E2B1, Themes E771, Lock screen EE3F, Start E1E4, Taskbar E147, Fonts E185;
Text input keeps the keyboard symbol because F741's 40 px mask blurs when
resampled to 30 px at 150 %, see Verification) and descriptions; Device
usage omitted (owner exclusion).

Colors: the bezel preview (15 px above the first card, as on Background),
Choose your mode, Transparency effects, then the Accent color expander
holding Recent colors (new preference `ACCENT_RECENT`: up to five Windows
color indices, most recent first; a new account has the default blue),
the 48 colors with Windows' accessible names (75 px swatches at an 80 px
pitch at 150 %, so 50/53 epx, nine to a row, 58 px in; the chosen one
gets a 2 px text-colored frame and a 20 px check badge), Custom colors
(View colors, currently unavailable) and both "Show accent color"
toggles, as 26H2 nests them. Contrast themes still map the swatches onto
the system colors.

Page top (all pages): the 150 % captures put the scrolled content's clip
at 158 px (105 epx) and the first block at 166 px (111 epx) on card,
header and hero pages alike. Doors had 120 and 124; both moved, which
lifts every page 13 px to Windows' position.

Assets: the JPEG originals ship as they are (26 MB) instead of lossless
conversions (about 200 MB for 22 images at 3840x2400); the window server
gains a bounded baseline/progressive JPEG decoder (an approved upstream
crate, decoded once per wallpaper change into the existing cache). The four
images Doors already has (Bloom, dark Bloom, Glow img20, Sunrise img28) keep
their pinned lossless forms and provenance.

## Material and colour

- Settings' background is Mica: the wallpaper, heavily blurred, under the
  dark or light Mica tint and luminosity layers. The owner's captures over
  the dark Bloom give the target pixels; Doors computes the same from the
  same wallpaper, so the comparison is exact, not by eye.
- Accent-dependent colours (selection indicator, toggles, links, the
  accent-tinted title bar option) follow the accent palette already in
  `door-design-tokens`.
- Implemented (owner report, 12 November 2026: the flat grey background
  was wrong): Settings asks the window server for Mica
  (`Application::mica`, `SET_MATERIAL` `MATERIAL_MICA`) and paints with
  materials on. It clears its page and title background and turns each
  fill into the white or black veil that resolves to its calibrated colour
  (`settings/theme.rs` `mica`: a dark card is white at about 5 %, as
  WinUI's CardBackgroundFillColorDefault). Text, accent, strokes, dialogs
  and menus stay opaque; contrast themes keep their solid system colours,
  and the compositor shows the solid fallback (#202020 / #F3F3F3) while
  Settings is not the active window or transparency is off. Guest probes
  of the Settings background compare with the compositor's own Mica at
  that point over the same wallpaper (`door_graphics_core::MicaField`,
  xtask `desktop/mica.rs`), so they stay exact.

## Pages

Progress (12 November 2026): Doors' 64 pages plus 98 generated from the
outlines cover 147 of the 163 Windows pages; the rest are excluded by the
owner's rules. Reworked against the captures so far: Personalization,
Background, Colors, Themes, Lock screen (preview with time and date),
Start (Recommended / All / Other plus Doors' Layout section; Folders is
its own page with the live folder toggles), Taskbar (Related: Date &
time, Notifications), Fonts, Text input, Power & battery (the layout
Windows shows without a battery), Notifications, Storage (Advanced
storage settings), and the System, Privacy & security, Accounts,
Accessibility and Display landings. Descriptions that said
"Unavailable: <reason>" now carry Windows' own text (or none); the control
says "Currently unavailable". The comparator's remaining differences are
the owner's own data (apps, history, networks, devices), Macrohard-account
services and excluded features.

160 pages in 13 categories (System 30, Privacy & security 36,
Accessibility 19, Personalization 12, Bluetooth & devices 11, Accounts 11,
Network & internet 10, Time & language 9, Apps 9, Windows Update 7, Gaming
4, Home, Advanced sharing). Doors has 58 of them plus the unified legacy
tool pages. Work per page:

1. Structure from the outline: groups, cards, expanders, controls in order,
   with Windows' titles and descriptions.
2. Geometry from the screenshots and UIA rectangles (card heights, gaps,
   control widths, expander bodies), checked by the existing reference
   painter tests at 100/150/200 %.
3. Behaviour: each control bound to its Doors provider; settings without a
   provider on Doors (Xbox, Recall, Copilot, phone link, cellular) are shown
   disabled, never as working stand-ins, per the catalog's rule.
4. Navigation: breadcrumbs, back stack, deep links (`ms-settings:` for every
   page), search over the page and setting index.

Order: category landing pages and Home first (most visible), then System,
Personalization (with themes), Bluetooth & devices, Network & internet,
Apps, Accounts, Time & language, Gaming, Accessibility, Privacy & security,
Windows Update.

## Animation and behaviour

Measured (12 November 2026, `reports/artifacts/reference-26h2/motion/settings-page/`,
UI Automation selection on the rail, 13 ms frames, three runs): the old
page fades out over about 150 ms on an accelerating curve, then the new
page rises 140 epx (210 px at 150 %) to its place on (0.1,0.9,0.2,1) over
300 ms, opaque from its first frame; the page title switches at once.
Doors implements the rise as `motion.page_enter` (the page marker
`DOOR_P12 SETTINGS PAGE` is written when the page has settled, so guest
captures see the final layout); Reduce motion and deep links skip it.
Not yet reproduced: the old page's fade-out (Settings would have to paint
the previous page after switching). The expander and combo captures fell
outside the sampled region; those keep the WinUI control durations
(`motion.fast`/`motion.normal`) until captured.

Page entrance (the content slides up and fades in), the navigation
selection indicator, expander open/close, toggle switch thumb, combo box
drop-down, and the breadcrumb transition: measured from screen recordings
on the owner's PC like Start's (VIS-01), then added to `door-design-tokens`
with their provenance.

## Verification

- Host: reference painter tests per page (structure and geometry against
  the outline and capture), token tests, theme application tests.
- Guest: the Settings scenarios (settings-reference, p14-settings,
  display-01, vis-01) extended to the new pages, and a theme scenario
  (apply each theme, check wallpaper, mode and accent).
- 150 % raster check (`fractional_apps.rs`): no half-alpha line pairs on
  Home, Devices & drivers, Personalization, Background and Storage.
  Known limit: Segoe Fluent symbol masks ship at 20/40/60 px and are
  area-resampled to 30 px at 150 %, which blurs some straight strokes
  (F741 shows it). Native 30 px masks per symbol are the follow-up that
  removes the limit.
- Over Mica every committed pixel must be premultiplied (no channel above
  alpha), or the compositor refuses the commit. `fractional_apps.rs`
  `settings_over_mica_commits_premultiplied_pixels_at_every_scale` checks
  Home, System, Apps and Personalization at 100/150/200/300 %; the icon
  exporter clamps each channel to alpha (twelve exported icons had one
  above it and Settings failed at 300 %).
- Pre-gate guest batch (13 November 2026): desktop-wide (18 wallpapers,
  36 appearance words, dark and light Settings over Mica),
  desktop-resolutions, settings-reference, p14-settings and desktop-a-apps
  (the mode and picture committed through the typed registry, observed
  live by Files, Notes and Terminal) on the final image.

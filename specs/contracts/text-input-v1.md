# Keyboard input and text input (v1)

Status: implemented and host-tested (P12 workstream I, stage 1). App adoption
(TextBox, Notes, Terminal), the Settings keyboard page, the taskbar input
indicator and the candidate-window overlay call site are stage 2.
Requirements: master §53 (input routing, layouts, IME), §109 (international
input) and gate TEXT-01.

## Layers

| Layer | Owner | Carries |
|---|---|---|
| Device ABI `door_abi::graphics` KEY | kernel `keyboard.rs` | physical set-1 code, sided modifiers, locks, repeat, legacy US ASCII |
| Layout translation | window server, `door_keyboard_layouts` | Windows virtual key, USB HID usage, Unicode text, dead keys, AltGr |
| Routing | window server, `door_window::text_input::Broker` | legacy INPUT key event, or ordered text-input records |
| Composition | IME process `door-ime` (manifest 16) | preedit, commit, delete-surrounding, candidates, mode |

A hardware key is never a character: the kernel reports physical keys only;
the session layout gives them meaning in the window server.

## Device KEY event (ABI v1 extension)

`code`: set-1 code, `0x100` for E0-prefixed keys, `0xF1` Hanja / `0xF2`
Hangul (make-only: one DOWN, never an up). Fake shifts (E0 2A/E0 36) and
controller bytes (00, FA, FE, EE, FC, FF) are not keys.

`flags` (state after the event): `DOWN 1`, `SHIFT 2`, `CONTROL 4`, `ALT 8`
(unchanged), `REPEAT 16` (make for a key already down), `RIGHT_ALT 32`,
`LOGO 64`, `CAPS_LOCK 128`, `NUM_LOCK 256` (starts on), `SCROLL_LOCK 512`,
`LEFT_SHIFT 1024`, `RIGHT_SHIFT 2048`, `LEFT_CONTROL 4096`,
`RIGHT_CONTROL 8192`, `LEFT_ALT 16384`, `LEFT_LOGO 32768`, `RIGHT_LOGO
65536`. A sided bit implies its aggregate; REPEAT implies DOWN; `x`/`y` are
zero. Locks toggle on the first make only.

## Legacy window key code (unchanged contract, extended bits)

`INPUT` event detail 1, `a = code | (flags & PACKED_FLAGS) << 9 | ascii <<
16`, `b = down`. `PACKED_FLAGS = SHIFT|CONTROL|ALT|REPEAT|RIGHT_ALT|LOGO`
(bits 10..15); bit 9 and bits 23..31 are zero. The ASCII byte now comes from
the session layout: printable ASCII, Ctrl+letter control codes of the
layout's letter, or the Enter/Tab/Backspace/Escape/Delete codes. Text typed
through AltGr clears CONTROL/ALT and sets RIGHT_ALT so shortcuts never fire
on it. Non-ASCII text is not representable here: use text input.

## Layouts and switching

Layouts (Windows KLID): US 00000409, US-International 00020409, United
Kingdom 00000809, Italian 00000410, German 00000407, French 0000040C,
Japanese 106/109 00000411 (with the Japanese IME), Korean 00000412 (US
positions, Type 1 Right Alt = Hangul, Right Ctrl = Hanja). Chinese uses US.
Each key defines base/Shift/AltGr/Shift+AltGr characters, CAPLOK and
CAPLOKALTGR, and dead keys; AltGr is Right Alt or Ctrl+Alt on AltGr
layouts; a dead key followed by Space gives its spacing character, by an
unmatched character both characters; Escape/Enter/Tab/Backspace cancel it.
Num Lock selects keypad digits (Shift inverts it). Ctrl/Alt chords produce
no text.

Input methods (`door_keyboard_layouts::methods::METHODS`): 1 en-US US,
2 en-US US-International, 3 en-GB UK, 4 it-IT Italian, 5 de-DE German,
6 fr-FR French, 7 zh-CN Microsoft Pinyin, 8 ja-JP Microsoft IME, 9 ko-KR
Microsoft IME. Hotkeys (Windows 11 defaults): Win+Space next method
(Shift: previous; consumed with its release, Start stays closed); Left
Alt+Shift next language and Ctrl+Shift next layout of the language, both
on a clean release of the chord; Hangul, Hankaku/Zenkaku (Alt+`) and Shift
alone (Pinyin) toggle the IME mode inside the IME.

Per-user preference word (`Preferences`, setting `InputMethods`): six 4-bit
method ids in order (bits 0..24, no gaps or duplicates), current index (bits
24..27), bit 27 "different input method for each app window" (stored, not
yet applied). Default `0x00000001` (US).

## Opcodes (`0x5240..=0x527F`, on the door.window channel)

| Op | Value | Body | Who |
|---|---|---|---|
| ENABLE | 0x5240 | window u64, content u32, hints u32, caret Rect | owner of the window; reply serial; at most 16 contexts per application (64 per server), refused while its queue is overflowed and unpolled; re-ENABLE retires the old context (composition cancelled, preedit cleared) |
| DISABLE | 0x5241 | window u64 | owner; the IME commits the composition to the retired context, as on a focus change |
| CURSOR | 0x5242 | window, serial, 0 u32, caret Rect (content-local physical px) | owner |
| SURROUNDING | 0x5243 | window, serial, cursor u16, anchor u16, UTF-8 ≤ 224 B | owner; refused for secure content |
| RESET | 0x5244 | window, serial, mode (0 cancel, 1 complete) | owner |
| POLL | 0x5245 | empty; reply packed records ≤ 240 B, WouldBlock | apps and the IME |
| STATE | 0x5246 | empty; reply preferences, method, mode, flags (bit 0 IME registered, bit 16+n method n without candidates) | any |
| SET_METHODS | 0x5247 | preferences u32 | workspace apps (Settings) |
| SELECT | 0x5248 | index u32 | workspace apps (indicator flyout, Settings) |
| SET_PREFERENCES | 0x5249 | preferences u32 | Init, session 0 (sign-in) |
| IME_REGISTER | 0x5260 | version 1, methods mask (bit n handled; bit 16+n: composes without its candidate dictionary); reply current method | IME manifest only, one per session |
| IME_PREEDIT | 0x5261 | serial, cursor u16 (0xFFFF hidden), count u8, 0, attrs × (start u16, end u16, style u8, 0), text ≤ 172 B (one PREEDIT record with 8 attributes; the IME cuts on a grapheme boundary) | IME, active context; an empty clear also reaches a just-retired context |
| IME_COMMIT | 0x5262 | serial, 0, text ≤ 224 B = one COMMIT record (clears preedit; longer commits are split by the IME on grapheme boundaries) | IME, active or just-deactivated context |
| IME_DELETE | 0x5263 | serial, before u32, after u32 (bytes) | IME, active context |
| IME_KEY_DONE | 0x5264 | key serial, verdict (1 forward, 0 consumed) | IME, in order |
| IME_CANDIDATES | 0x5265 | serial, selected, count ≤ 9, page, pages, flags (1 vertical), 0×3, lengths, UTF-8 items ≤ 48 B; a page holds at most 9 items whose body fits one request | IME; count 0 hides |
| IME_MODE | 0x5266 | mode (0 direct, 1 native, 2 katakana, 3 full-width) | IME |

Content types: 0 normal, 1 password, 2 URL, 3 number, 4 e-mail, 5 terminal,
6 PIN. Hints: 1 no prediction, 2 private (no learning), 4 multi-line.

## Records (POLL)

`[len u8][kind u8][0 u16]` + payload, packed back to back. Applications:
`1 KEY` (window, timestamp, code, ABI flags, legacy code, VK u16, HID usage
u16, text length, status, text ≤ 16 B UTF-8; status 1 dead key pending, 2
composed, 4 forwarded by the IME, 8 AltGr), `2 PREEDIT` (empty clears),
`3 COMMIT`, `4 DELETE`, `5 DISABLED` (1 queue overflow, 2 locked). IME:
`16 ACTIVATE` (serial, content, hints, method, scale percent, caret in
output physical px), `17 DEACTIVATE`, `18 SURROUNDING`, `19 CURSOR`,
`20 IME_KEY` (key serial, context serial, key), `21 METHOD`, `22 RESET`.

## Capabilities and events

HELLO bit 26 `CAP_TEXT_INPUT` (applications) and bit 27 `CAP_IME` (granted
to manifest 16 only). Such clients receive one coalesced window event
`0x130 PENDING` (a = queued records) when records wait; they then POLL.
`WindowClient::poll` decodes it for those clients only. Event kinds
`0x130..=0x13F` are reserved here.

## IME process lifecycle

Init launches `door-ime` (manifest 16, session user, one thread, 48 MiB)
once the signed-in session is bound, restarts it with backoff (at most three
times in a row; an IME that ran 60 s earns the budget back) and retires it
with exit code -151 before the window service at
sign-out and shutdown (`DOOR_P12 IME LAUNCHED/EXIT`). It maps its
dictionaries through RESOURCE_OPEN ids 8 (Pinyin) and 9 (Japanese), granted
to manifest 16 only. Both are optional initfs entries: without one, its
method composes and commits the typed reading (Korean and kana need none)
and registers with the no-candidates bit, which Settings shows. Init applies
the user's `Setting::InputMethods` (id 6) with SET_PREFERENCES at bind;
Settings > Time & language > Typing persists it and applies it with
SET_METHODS.

## Routing rules

1. Lock screen, sign-in and shell keys are handled first; the hotkeys run
   before the shell's Win chords.
2. Every key reaching application dispatch is translated by the session
   layout. A focused window without an enabled context gets the legacy code.
3. With an enabled context all of its keys (downs and ups) arrive as KEY
   records, in order with PREEDIT/COMMIT/DELETE.
4. The key goes to the IME first when the current method is an IME, the
   IME is registered and responsive, and the context is neither secure
   (password, PIN) nor URL/e-mail/number (Windows opens those in direct
   input). The IME finishes keys in order; a key not finished within 30
   ticks (300 ms) is forwarded and the IME is bypassed until it answers.
   Keys forwarded while the IME still holds them (timeout, Win+Space,
   pending overflow, focus moving to a field without context) form an
   epoch: their composition is cancelled at the IME (or its late commit
   refused), the application's preedit is cleared, and IME output for
   those contexts is dropped until the IME finishes the epoch's last key or
   polls an empty queue, so no text arrives twice.
5. A focus change deactivates the context: the IME commits its composition
   to the old field (late commits reach the four most recently deactivated
   contexts). Locking cancels composition, clears the application preedit
   and refuses every IME request; nothing is sent to the IME while locked.
6. Secure contexts never store surrounding text and never reach the IME.
   Only the IME manifest receives surrounding text, activation and keys of
   other applications; only it may send preedit or commit. A client's
   context is bound to windows it owns. An application queue admits 96
   records of KEY and preedit data; on overflow that data is dropped and
   the application's contexts are disabled (`DISABLED` 1), while commits,
   deletions, disables and preedit clears are never dropped (17 records
   are reserved for them). A window that closes retires its context with
   the composition cancelled. When the IME exits, the active and recently
   retired contexts get a preedit clear, its pending keys are forwarded and
   none of its state survives into the next registration. The taskbar
   shows an IME conversion-mode glyph only while a registered IME serves
   the current method (otherwise direct input, e.g. 英).
7. Shell fields (Start search, Run) use a context owned by the window
   server itself rather than a client: it is enabled while the field has
   keyboard focus and routed by the same rules (the IME sees it as an
   ordinary plain-text context). Commits and preedit are applied in place by
   `door_window::text_edit` (grapheme-cluster backspace, delete and caret
   movement; preedit drawn inline with a dotted underline). v1 limitation:
   closing Start with the pointer during composition drops the preedit
   instead of committing it.
8. Markers: `DOOR_P12 TEXT_INPUT METHOD id= ime=` per method change and
   `DOOR_P12 CANDIDATES SHOWN` (frame, caret) per opened candidate window
   (window server, 24 each per session); `DOOR_P12 IME COMMIT n= bytes=`
   (IME, 64 per process).

## Candidate window

Drawn by the window server as a shell overlay at the active caret
(`door_window::text_input::candidate`): Windows 11 metrics at the output
scale (36 px horizontal bar for Pinyin with page chevrons, 32 px rows for
Japanese with an accent selection pill and page counter, 8 px corners),
below the caret or above it at the work-area edge, shell palette including
contrast themes, text through `TextSystem` with the method's locale.

## Client library

`door_service_client::text_input`: `WindowClient::text_input_enable/disable/
cursor/surrounding/reset/poll`, `input_state`, `set_input_methods`,
`select_input_method`, the `ime_*` calls, `apply_input_methods` (Init), and
the reference `TextField<N>` that applies records (Unicode keys, VK editing,
preedit at the caret, commit over the selection, delete-surrounding,
password masking).

## Limitations (v1)

Windows IMM/TSF adapters, compose-key sequences beyond dead keys, Alt+numpad
codes, Kana-lock typing, Hanja conversion, per-window input methods, IME
reconversion from surrounding text, keyboard LEDs and half-width katakana
glyph forms are not implemented. Dictionaries are acquired by
`cargo xtask ime-dictionaries --fetch --pin` (owner approval required) and
mapped as on-demand resources; without them Pinyin and Japanese compose
without candidates.

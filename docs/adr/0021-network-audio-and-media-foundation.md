# ADR 0021: P15 network, audio and media foundation

Status: adopted 3 November 2026 (P15); amended 11 November 2026 after the
P15 critic review (items 1, 4, 5, 7 and 8 below; the original text of
those items described the state of 3 November). Plan:
`docs/design/p15-plan-20261101.md`.

## Context

P15 (specification :2650, sections 27 and 28) adds networking, Winsock, DNS,
secure transport, audio, multimedia and capture, with acceptance CONNECT-A
and SOUND-A (:4223-4224) and exit evidence of controlled network and sound
scenarios with cancellation, denial, malformed input and device loss. P15
cannot pass before P12-P14.

Owner decisions: 1 November 2026, all of P15, with working network and media
settings and GUI; 2 November 2026, implement P15, P16 and P17 now and close
each phase formally once P12-P14 pass ("implement all, gate later"), every
VPN type its real protocol, one read-only critic per phase close, Rust i686
targets and boringtun (host-only WireGuard test peer) may be downloaded.

## Decision

1. **Schema.** P15, P16 and P17 are explicit phases in `tools/xtask`
   (parse, cases, `check_id`, serial matching, demo title); unknown phases
   fail closed from P18. `verify --phase P15` runs every P14 command, guest
   case, desktop workflow and suite, then: clippy of `door-network` and
   `door-audio` on the guest target, clippy of the P15 DLLs (ws2_32,
   iphlpapi, dnsapi, winhttp, wininet, winmm, dsound, mmdevapi, with ole32
   and shell32) on the PE target, host suites of the services, `door-tls`,
   `door-http`, `door-windows-inet`, `door-firewall`, `door-wireguard`,
   `door-pac` and each DLL, and the desktop scenarios `p15-desktop`,
   `p15-connect` (CONNECT-A), `p15-sound` (SOUND-A, exclusive: it measures
   real-time playback and timer periods), `p15-capture`, `p15-dsound`,
   `p15-midi`, `p15-wasapi` and `p15-vpn`.
2. **Services.** `door-network` (smoltcp over the polled VirtIO NIC) is the
   only holder of the NETWORK authority; `door-audio` (virtio-sound, mixer,
   sound scheme) the only holder of AUDIO. Clients reach them over
   `door.network` and `door.audio`; payloads stay within the 256-byte IPC
   body, larger data moves in bounded parts (datagram staging) or sections
   (audio).
3. **Windows DLLs.** `ws2_32`, `iphlpapi`, `dnsapi`, `winhttp`, `wininet`
   and `winmm` are original Rust DLLs importing only ntdll; application
   memory crosses only through `DoorUserCopy`, in chunks of the role's
   copy bound (4 KiB for console and windowed applications). Classic
   Winsock exports keep Windows' ordinals because SDK import libraries bind
   them by ordinal.
4. **Transport.** WinHTTP and WinINet share `door-windows-inet` (resolution,
   TCP with deadlines, TLS 1.3 and 1.2 through `door-tls` with Windows
   certificate error flags, cookies, proxies, PAC/WPAD, Basic, Digest,
   NTLM and Negotiate authentication, decompression); WinINet adds FTP and
   the URL cache. What is not implemented fails with Windows error codes,
   never with fake success.
5. **Multimedia.** winmm's waveOut maps each device to a door.audio stream
   and reads an application buffer only when it hands it to door.audio
   (the buffer is the driver's until WHDR_DONE), so no buffer is copied
   through the heap. Callbacks run outside every lock. Behaviour follows
   measurements of Windows 11 26H2 on the owner's PC (soundprobe in silent
   mode): TIME_MS positions are answered in bytes, capabilities and error
   texts match, dwSupport claims only what door.audio does. waveIn
   (capture), MIDI (a software synthesizer), mixers, auxiliary devices and
   MCI follow the same rules; dsound.dll and MMDevApi.dll (WASAPI) sit on
   the same door.audio streams. MessageBeep and MessageBox play the
   sound-scheme events. When door.audio loses its device, playback streams
   end (winmm hands every buffer back done and answers
   MMSYSERR_NODRIVER; WASAPI answers AUDCLNT_E_DEVICE_INVALIDATED) and the
   device is probed again every second.
6. **Diagnostics.** A panicking Rust compatibility DLL still fast-fails with
   0xC0000409, after writing its source location to the console process's
   standard error.
7. **VPN.** WireGuard is the VPN, as its real protocol (`door-wireguard`,
   tested against boringtun as a host-only peer). IKEv2, SSTP, L2TP and
   PPTP are not offered (owner decision of 8 November 2026); no VPN type
   claims to connect without its protocol.
8. **Firewall and isolation.** Windows Defender Firewall's rules
   (`door-firewall`, the registry form) are enforced by `door-network` on
   frames before the stack sees them. Whatever the firewall's state,
   frames from another host never come from or go to the loopback
   interface's addresses or from this host's own; a rule condition the
   model cannot evaluate fails closed (an allow rule never matches, a
   block rule applies as if the condition held). The stack's sequence
   numbers, ports and DNS IDs come from the kernel's random generator.

## Consequences

- CONNECT-A (42 steps) and SOUND-A (15 steps) run ordinary MSVC-built
  programs in the guest; their Windows reference runs on the owner's PC
  open no listening socket and make no sound.
- The phase stays `in_progress` until P12-P14 pass and its gate, records and
  critic are complete.

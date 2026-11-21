# ADR 0025: Out-of-process COM over packets Init relays

Status: adopted 11 November 2026 (P16 M6). Plan and progress:
`docs/design/p16-plan-20261102.md` ("M6 oleaut32, type libraries,
out-of-process COM"). Subject to the P16 critic review.

## Context

P16 needs out-of-process COM: LocalServer32 activation, standard
marshaling of interface pointers between processes, calls with in/out
arguments and error info, callbacks into an STA, and the failures a
client sees when the server dies. On Windows this runs on rpcrt4's
ncalrpc (ALPC ports), with NDR stubs that MIDL generates or the
universal marshaler builds from a type library. The P16 plan named
rpcrt4's ncalrpc as the transport. Doors' kernel IPC does not fit a
per-connection port model: a channel end cannot be handed to another
process (only the process that created it can use it), and the kernel
has eight ports in all, so one port per COM server or per connection is
not available.

## Decision

1. **Init relays COM packets.** A process REGISTERs an endpoint (`OLE<pid>`
   for its apartments, `OLECLASS-<clsid>` for a class it serves), a
   client CONNECTs to an endpoint and receives a link, SEND queues a
   packet of at most 232 bytes on a link (4 queued per side, 24 in Init
   in all; WouldBlock beyond), and POLL hands a process its next event
   (a new link, a packet, a peer that closed). All state is keyed by the
   process's existing connection to Init and is dropped when that
   connection goes, so a dead server's links report closed peers.
   Endpoints are scoped to their owner's session, user and elevation
   (amended 11 November 2026 after the P16 critic): a process finds,
   connects to and activates only endpoints of the same identity, an
   object exporter `OLE<pid>` is registered by that process only, and each
   connection holds at most 4 endpoints, opens at most 4 links and has at
   most 8 packets waiting. OIDs, and the IPIDs made from them, are random.
2. **The COM layer above the transport is Windows'.** OBJREFs use the
   MS-DCOM layout (OXID = process, OID, IPID); ole32 keeps one proxy
   manager per COM identity; stubs and proxies come from each
   interface's IPSFactoryBuffer, and oleaut32's universal marshaler
   builds them from the registered type library (dual and
   oleautomation interfaces, IDispatch). Activation follows the
   registry: Init's broker says whether a class's server runs, the first
   client starts it with `-Embedding`, and CoRegisterClassObject
   publishes the class endpoint. STAs serve calls from an
   "OleMainThreadWndClass" message-only window; the MTA from one
   dispatcher thread.
3. **rpcrt4.dll is not provided.** MIDL /Oicf proxy-stub DLLs (NDR
   format strings) are not supported; interfaces marshal through type
   libraries or IDispatch.

## Consequences

- No kernel change and no new ports: COM shares Init's request channel
  that every process already has, and its lifetime rules come for free.
- Large arguments travel as several packets; the 232-byte packet and the
  queue limits bound memory in Init, at the price of throughput.
- An idle STA polls its links from a 10 ms timer, so a cross-process
  call to an idle server costs at least one timer period.
- COMOOP-A (register, activate, calls, in/out, error info, dispatch,
  callback, apartments, crash, relaunch, unregister) and OLEAUT-A are the
  guest evidence. OLEAUT-A's expected values come from its probe run on
  Windows 11; COMOOP-A registers a COM server, so it runs in the guest
  only and checks Windows' documented behaviour.
- Not provided: CoImpersonateClient and the caller's identity in a call
  (a server sees only same-identity callers), CoInitializeSecurity
  descriptors (accepted, not enforced beyond that scoping), cross-session
  and cross-user activation (RunAs/AppID identities).
- Revisit if a later phase raises the port limit or lets channel ends
  move between processes (both would need the owner's approval): a
  direct per-server channel would remove the relay and the polling.

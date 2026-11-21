# ADR 0006 — Native scheduler ownership, waits and context retirement

Status: implemented P04 metadata mechanisms; actual scheduling, SMP and enabled
register-state qualification remain tied to the phase's guest gate and critic.

`kernel/src/task` contains an allocation-free native state core for 16 tasks,
4 online CPUs, 32 synchronization objects and wait-any sets of up to 4 objects.
It neither creates host threads nor executes a context switch. The kernel runtime
holds one IRQ-safe SMP lock while calling it. On interrupt entry the runtime saves
the real architectural context and passes its address to `schedule`; the returned
context identifies the next real task. Context addresses are metadata here, not
permission to dereference an arbitrary caller-provided pointer.

Tasks pass through explicit New, Ready, Running, Blocked, Suspended, Exiting and
Terminated states. `create` builds New metadata; `start` publishes it. Each CPU has
a bounded ready queue. Selection scans eligible entries for native priority 0–7,
highest first, then FIFO registration order. A ready task gains one scheduling
priority level per eight monotonic ticks, capped at seven; equal-priority tasks
rotate. Ready membership belongs to exactly one CPU queue. Affinity restricts
dispatch and load balancing; another CPU may steal an eligible ready task, with
actual migrations counted. Priority numbers are original native policy and do not
claim Windows priority compatibility or hard real-time service.

Saving a context and selecting another task do not retire the old context: an IRQ
epilogue can still write the outgoing stack. `schedule` marks that task retiring,
removes it from CPU-current membership and returns a CPU/epoch retirement ticket.
It cannot enter any ready queue or be reaped during this interval. The architecture
switch changes RSP to the selected context first, ceases all access to the old stack,
and calls the runtime commit hook while the same scheduling lock is still held.
Only then may `acknowledge_retirement` publish the old task as ready, finalize its
termination or leave it blocked/suspended. A CPU cannot schedule again with an
unacknowledged ticket; stale or duplicate tickets fail. If no better ready task
exists, a still-runnable current task can remain current without a retirement.

Exit requests are distinct from terminal results. A running or retiring task remains
Exiting until context retirement. Mutex abandonment also waits for that point, so
another CPU cannot acquire a mutex while its former owner still executes the old
epilogue. Terminated tasks expose a stable result; explicit `reap` returns their last
saved context once and invalidates the task ID. Task and object IDs carry a slot and
64-bit generation. A generation never wraps: a slot at its maximum generation is
permanently excluded from reuse. FIFO sequence numbers are renormalized over the
bounded live metadata before overflow, preserving order. Diagnostic counters saturate.

The runtime must retain its scheduler lock across availability inspection, waiter
registration, Blocked transition, context save and architecture retirement commit.
Thus a signal cannot be lost between checking an object and sleeping. The core also
preserves wake obligations when a completion is injected in either transition window:
it records the outcome but defers ready publication until the outgoing context is safe.
Exactly one of signal, timeout, cancellation or object closure completes an ordinary
wait. Subsequent causes cannot replace its recorded result. An already available
object wins over an expired deadline observed during the initial locked wait call;
otherwise the first completing operation under the lock wins.

Manual events retain their signaled state until reset and complete all registered
waiters. Auto events retain one signal and release one waiter. Semaphores consume one
count per completed wait; releases reject zero, arithmetic overflow and exceeding the
declared maximum without changing state. Wait-any checks input order and consumes only
the first ready object. Waiters are serviced by effective priority and then FIFO order.
Every object in a wait set is generation-checked before state changes; duplicates and
oversized sets fail. Conditions are intentionally excluded from ordinary wait-any.

Native mutexes are nonrecursive and owner checked. Waiting on one's own mutex fails;
a provable single-mutex wait cycle is rejected. Priority donation propagates through
mutex wait chains for at most 16 bounded passes and is recomputed when ownership or
waiters change. Wait-any may resolve through another object, so no blanket deadlock
claim is made for arbitrary application resource graphs. Owner exit sets abandonment;
the next successful acquirer receives an abandoned flag and actual ownership.

A condition wait requires an owned mutex. It atomically registers on the condition
and releases that mutex under the scheduler lock. Conditions do not latch signals:
callers maintain and recheck their predicate under the associated mutex. Signal,
timeout, cancellation or condition closure selects the condition outcome once, then
the task waits for mutex reacquisition before it can run. That reacquisition cannot
be timed out or cancelled a second time. If the mutex itself is explicitly closed,
the wait completes Closed without ownership; this acquisition failure supersedes
the earlier condition outcome. Closing an object unpublishes its generation and
completes relevant waiters before its slot can be reused.

Suspension prevents ready publication but does not erase a wait or a completion.
Resume restores New, Blocked or Ready according to actual prior work. Suspending a
running task requests rescheduling and still uses the retirement handshake. This
primitive does not make arbitrary suspension of a mutex owner safe for application
liveness; higher-level process/debug policy must control that use. Terminating a
blocked task removes its registration and releases owned mutexes only after any
live context has retired.

The monotonic clock accepts equal or increasing tick values and rejects reversal.
Timeout processing is bounded by the 16-task limit. Tick resolution and actual APIC
delivery are runtime/platform responsibilities. This phase does not implement native
user executors, Windows wait-all/APCs/alertable waits, user process handles or a
general asynchronous I/O lifetime policy; those depend on later protected execution
and Windows semantics.

Host tests exercise priority/FIFO/aging, four-CPU metadata stress, explicit migration
retirement, signal before registration, signal during both retirement windows, all
24 orderings of signal/timeout/cancel/close, stale generations, semaphore accounting,
transitive donation/cycle rejection, abandoned mutexes, condition reacquisition,
suspension, stable terminal results and bounded exhaustion. The guest gate must
separately demonstrate preemption, independent register/TLS/SIMD patterns, real AP
startup, cross-CPU visibility, waits, migration and remote translation retirement.

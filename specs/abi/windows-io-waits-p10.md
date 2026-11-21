# P10 event, APC and asynchronous file profile

This is a bounded original implementation, using native ABI1 handle objects,
thread retirement and the authenticated storage service. It is a development
contract until the integrated P10 gate qualifies the independent fixture.

Unnamed CreateEventW supports manual and auto reset, SetEvent/ResetEvent and
generation-safe native close. Single waits support native events and threads.
Multiple waits support wait-any with 1–16 distinct handles, selecting the lowest
signalled array index. Wait-all, named events/security descriptors, mutexes and
semaphores are explicit unsupported requests. Each blocking native call retains
its kernel object while blocked; close cannot destroy an object under that call.
Public file handles are not wait objects in this bounded profile; requests with
NULL hEvent use GetOverlappedResult's private worker wait for completion.

QueueUserAPC owns a FIFO queue of eight callbacks for each registered Windows
thread. Registration precedes publication of its native child, covering the
child-before-parent-return race. Exit discards undelivered APCs. Close removes the
public handle mapping without killing a live thread. GetCurrentThread's -2
pseudo-handle resolves only to the caller. Normal waits and Sleep do not dispatch
APCs. SleepEx and alertable event/thread/wait-any waits drain a snapshot outside
metadata locks and return WAIT_IO_COMPLETION. Callback state uses the target's
own FS/GS. Polling adds at most one native 10ms tick of idle wake latency; timer
scheduling and callback execution can add latency. There is no kernel APC or
forced asynchronous callback injection. Callbacks queued by callbacks remain for
the next alertable dispatch.
Registry acquisition has a one-second deadline. A process-shutdown callback
encountering metadata abandoned by a terminated sibling fails explicitly rather
than waiting forever. Queueing through a public thread handle also verifies its
native stable exit record before accepting a callback.

CreateFileW accepts FILE_FLAG_OVERLAPPED. This profile owns one process-wide
request, at most1024 bytes, with an increasing nonwrapping generation. Submission
captures descriptor/offset/write bytes and checks read-output storage; no Rust
reference to application memory survives a syscall. A private native worker uses
owned buffers and serializes authenticated storage IPC with synchronous file
operations. It does not execute application DLL thread callbacks. File position
is unaffected; the captured OVERLAPPED offset selects the transfer.

Submission returns FALSE/ERROR_IO_PENDING. A duplicated native event retains
completion authority if its caller handle closes. Completion copies read bytes
through USER_COPY, publishes one terminal Internal/InternalHigh record and then
signals/releases the event. A later unmap produces a captured-copy error, never
an unchecked write. The caller must keep OVERLAPPED/read storage alive until
terminal completion, as on Windows. No range is promised pinned until completion.
Cancellation marks the captured generation atomically without waiting for the
storage IPC lock. The worker observes cancellation between transfer chunks;
an operation already committing may complete successfully. CancelIoEx after terminal
completion returns NOT_FOUND. GetOverlappedResult supports wait/query/repeated
terminal queries. Closing the file cancels queued work, joins its worker outside
the file metadata lease and closes the service handle after worker retirement.
Close marks the file closing before dropping the lease, preventing a concurrent
submission from starting during drain. A failed drain permits close retry while
other new operations remain denied. Concurrent completion observers duplicate a
WAIT reference under the lease and join that retained kernel handle outside it;
one observer closing the original handle cannot invalidate another's wait.
Only retirement permits request-slot reuse. A concurrent outstanding request
receives NOT_ENOUGH_MEMORY; exhausted native thread quota also fails explicitly.
There is no IOCP, completion routine, ReadFileEx, direct/unbuffered IO or arbitrary
offset-sentinel policy in this profile.

The independent SDK C source io_waits.c exercises auto/manual events, wait-any,
worker/current-thread APC delivery and deferral, FIFO ordering, stale handles,
real pending reads of the durable P10 file, cancellation-versus-completion,
repeated completion, retained event authority and file-close draining. Its reads
do not mutate the durable image, preserving the separate reopen zero-write check.
Eight requests also run two simultaneous ordinary Windows completion observers,
checking both return the same1024-byte terminal result while retirement races.
The portable request model tests duplicate terminal rejection, stale completion,
generation exhaustion, cancellation and reuse only after retirement. Host model
tests alone do not establish guest compatibility.

Contract references: master104/135; Microsoft Win32 CreateEventW,
WaitForSingleObjectEx, QueueUserAPC, SleepEx, ReadFile, GetOverlappedResult and
CancelIoEx. No Microsoft DLL implementation bytes are included.

Reviewed primary contracts: [ReadFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-readfile),
[GetOverlappedResult](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult),
[CancelIoEx](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelioex),
[QueueUserAPC](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-queueuserapc).

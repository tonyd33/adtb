---
title: The async runtime under the engine
pick: one
raised-by: [buffer/async/uring-fallback]
options:
  own-loop:
    title: The engine's own event loop per thread, swapped for the simulator in tests
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  tokio:
    title: tokio, with io_uring through tokio-uring
  thread-per-core:
    title: A thread-per-core io_uring runtime (monoio)
  split:
    title: Own loop in the engine core; tokio only in the server crate's networking
---
The pager submits and completes through one interface
(`buffer/async/uring-fallback`); morsel workers run plan pipelines
(`exec/parallel/morsel`); the simulator schedules threads
(`buffer/test/dst-threads/sim-scheduler`).

## project/arch/runtime/own-loop

```
trait Io { fn submit(&mut self, op: Op) -> Ticket; fn poll(&mut self) -> Option<Completion>; }
struct Worker<I: Io> { io: I, morsels: Queue, ... }   // I = Uring | Pool | Sim
```

No async runtime dependency; the simulator is the other `Io` and
scheduler.

## project/arch/runtime/tokio

Work-stealing; the server ecosystem (hyper, tonic) fits. Its scheduler
is not deterministic, so tests need a replacement runtime.

## project/arch/runtime/thread-per-core

monoio: io_uring or epoll on Linux, kqueue on macOS; tasks need not be
Send. Fits morsel workers pinned to cores.

## project/arch/runtime/split

The engine core is runtime-free over `Io`; the server's sockets run on
tokio and hand requests to the core. The simulator covers the core;
network faults are tested at the protocol boundary.

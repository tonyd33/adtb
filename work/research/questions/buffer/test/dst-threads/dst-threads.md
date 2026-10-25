---
title: Deterministic simulation needs control of every thread and timer
pick: any
raised-by: [buffer/test/dst]
options:
  single-thread-core:
    title: The engine core is a single-threaded state machine over an I/O interface
    links:
      - {rel: gains, to: engine/single-writer}
      - {rel: prior-art, to: prior/buffer-sim}
  sim-scheduler:
    title: A deterministic scheduler replaces the async runtime and OS threads in tests
  live:
    title: Live with partial determinism
    excludes: [buffer/test/dst-threads/single-thread-core, buffer/test/dst-threads/sim-scheduler]
---
Work-stealing runtimes and OS threads interleave nondeterministically,
so a failing seed does not replay. Parallel scans and a multi-threaded
pool are where the engine wants threads.

## buffer/test/dst-threads/single-thread-core

TigerBeetle: one thread executes everything; I/O completes through
the ring. Fits `engine/single-writer`; parallel query execution would
sit outside the simulated core.

## buffer/test/dst-threads/sim-scheduler

FoundationDB Flow's simulated executor; in Rust, madsim and turmoil
substitute runtime, time and network (unverified scope). Every
dependency's threads must go through it.

## buffer/test/dst-threads/live

Simulation covers single-threaded components; concurrency bugs found
only by stress.

---
title: A dependency's threads, clock or randomness escape the simulator
pick: any
raised-by: [project/arch/deps/allowlist, project/arch/deps/ecosystem]
options:
  no-ambient:
    title: Admit only dependencies that take no ambient time, threads or randomness
  wrap:
    title: Swap the dependency's runtime pieces for simulated ones in tests
  live:
    title: Leave dependency behaviour outside determinism
    excludes: [project/arch/deps/escape/no-ambient, project/arch/deps/escape/wrap]
---
`buffer/test/dst-threads/sim-scheduler` requires every thread, timer
and random draw to go through the simulator, or a seed does not
replay.

## project/arch/deps/escape/no-ambient

A rule checked by clippy `disallowed-methods` and `disallowed-types`:

```
# clippy.toml
disallowed-methods = ["std::time::Instant::now", "std::thread::spawn", "rand::thread_rng"]
```

Pure libraries (checksums, hashing, decimal arithmetic) pass; runtimes
do not.

## project/arch/deps/escape/wrap

shuttle ships wrappers for tokio, rand and std::sync; madsim replaces
tokio's runtime (unverified scope).

## project/arch/deps/escape/live

Nondeterministic seeds when a dependency is involved.

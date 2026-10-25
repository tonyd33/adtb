---
title: Dependency policy
pick: one
raised-by: [substrate/new-engine]
options:
  zero:
    title: No dependencies beyond the standard library
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
      - {rel: prior-art, to: prior/engine-fdb}
  allowlist:
    title: A short audited allowlist; each dependency justified
  ecosystem:
    title: Ecosystem crates where they fit (arrow-rs, tokio, parsers)
    links:
      - {rel: gains, to: exec/kernels/arrow-rs}
---
TigerBeetle has a zero-dependency policy apart from the Zig toolchain,
citing supply-chain, safety and performance risk. FoundationDB's
simulator cannot test code outside Flow, so it avoids external
dependencies (SIGMOD 2021). `exec/batch/arrow` fixes the Arrow memory
layout, not the arrow-rs crate.

## project/arch/deps/zero

Own parser, own Arrow layout, own io_uring bindings, own hashing.

## project/arch/deps/allowlist

```
# deny.toml
[bans] allow = ["io-uring", "libc", "crc32c"]   # each with a reason in DEPENDENCIES.md
```

Dev-dependencies (proptest, loom, cargo-fuzz targets) are separate.

## project/arch/deps/ecosystem

arrow-rs kernels, tokio for the server, a parser generator. Fastest
start; each crate brings its threads, allocations and MSRV.

---
title: Minimum supported Rust version
pick: one
raised-by: [substrate/new-engine]
options:
  pinned:
    title: A pinned toolchain, raised deliberately; the MSRV is that version
  rolling:
    title: A rolling MSRV at least six months old
  latest:
    title: Latest stable only
  nightly:
    title: Nightly, for unstable features
---
The embedded core is linked into other people's applications, whose
toolchains lag. The dev shell is devenv, which can pin a toolchain.

## project/arch/msrv/pinned

`rust-toolchain.toml` names one version; CI builds with it; bumps are
commits.

## project/arch/msrv/rolling

tokio: the new MSRV must have been released at least six months ago,
raised only in minor releases.

```
[package] rust-version = "1.85"
```

## project/arch/msrv/latest

No promise; embedders upgrade with the engine.

## project/arch/msrv/nightly

`allocator_api` for arena-typed collections, portable SIMD for
kernels. Embedders need nightly too.

---
title: Layer crates force public interfaces between tightly coupled layers
pick: any
raised-by: [project/arch/crates/by-layer]
options:
  internal-api:
    title: A doc-hidden internal API between engine crates, unstable by policy
  merge-coupled:
    title: Merge the coupled layers back into one crate
  live:
    title: Keep the public interfaces and their cost
    excludes: [project/arch/crates/coupling/internal-api, project/arch/crates/coupling/merge-coupled]
---
SSI locks name index pages, vacuum walks the union index and every
heap, the planner reads per-heap statistics and visibility maps. Rust
privacy stops at the crate; `pub(crate)` across crates does not exist.

## project/arch/crates/coupling/internal-api

`#[doc(hidden)] pub mod internal`; semver covers only the outer crate.

## project/arch/crates/coupling/merge-coupled

txn, storage and exec become one crate; the result is
`project/arch/crates/by-boundary`.

## project/arch/crates/coupling/live

Each layer's API is designed and versioned.

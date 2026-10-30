---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 1090
summary: "a storage failure writing a role registration's deployment facts is reported as an Internal rejection with Debug text"
branch: quality-fixes30
tags:
  - quality
  - engine
---

# A registration's storage failure reads as Internal

## Report

`RoleAuthority::register` writes the claim's deployment facts with `e.deployment.insert(..)` and maps every `catalog::Error` to `RejectReason::Internal { what: format!("{c:?}") }` (`crates/adtb/src/api/server/register.rs:76-80`). `Deployment::insert` fails with the roots heap insert's `catalog::Error::Heap(heap::Error::Page(..))`, a storage failure, or `Heap(TooLarge)`. Every other write in the same call goes through `Error::of_write`, which reports `write::Error::Page` as `Error::storage`. A full or failing device during a registration is therefore reported to the client as an engine bug.

Fix: map a page failure (`catalog::Error::Page` or `Heap(heap::Error::Page(..))`) to `Error::storage`, as `of_write` does, and keep `Internal` for the other variants. A sim test can witness it by injecting a write fault on the roots heap during `register`.

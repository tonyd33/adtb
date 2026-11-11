---
created-at: 2026-11-11
status: open
kind: bug
priority: 300
summary: "a fetch along a retag chain that meets an unreserved writer on a sibling constructor's heap page fails as a serialization failure and refuses nothing, so every retry meets the same damage"
tags:
  - quality
  - engine
  - class-damage-unreported
---

# A fetch meeting an unreserved writer on a sibling heap retries forever

## Report

`Store::fetch` (crates/adtb/src/exec/store.rs:892-895) passes the starting heap, `heap.id()`, to `Reads::saw` for every link `walk` returns. `walk` follows a retag chain into the other constructors' heaps of the sum, and each `Link.heap` names the heap that holds that link (heap/chain.rs:210).

When `sees` fails with `StatusError::Unreserved` on a link in a sibling heap, `Snapshot::saw` (store.rs:89-92) calls `Heap::refuse_unreserved` with `Slots::Versions(start heap)`. `names_writer` (heap/handle/mod.rs:95-99) requires the page's owner to equal that heap, so it is false, `refuse_if` refuses nothing and returns `Refused::Gone`, and `From<heap::Refused> for exec::Error` (exec/mod.rs:180-187) makes it `Serialization(ConcurrentWrite, [])`.

So a page holding proven damage (a writer xid never reserved) is never refused or marked, and the client receives a retryable serialization failure that every retry meets again. Found by reading; not reproduced.

Direction: pass the link's own heap, `saw((Pid(l.tid.page()), l.heap), ..)`. Test: plant an unreserved writer on a retagged-in version in another constructor's heap (as `with_unreserved_deleter` does) and fetch it through the starting heap's key, expecting the page refused (`Fatal Quarantined` with the heap's mark), not `Retry`.

Defect class: Damage reported as nothing: evidence of loss falls between two checks, each excusing it as the other's case.

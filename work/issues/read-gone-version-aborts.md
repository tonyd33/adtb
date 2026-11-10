---
created-at: 2026-11-10
status: open
kind: bug
priority: 350
summary: "a query read that meets a version gone under the latch, while refusing an unreserved writer, fails as a serialization failure with no xids instead of reading the page again"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# Bug: a read that meets a gone version aborts as a serialization failure

## Report

`AtSnapshot::saw` (crates/adtb/src/exec/store.rs:85-96) passes `Heap::refuse_unreserved`'s `Refused::Gone` through `?` into `impl From<heap::Refused> for exec::Error` (crates/adtb/src/exec/mod.rs:176-186), which builds `Error::Serialization(SerializationFailure::new(Cause::ConcurrentWrite, Vec::new()))`. Every exec read path through `saw` (`scan_past`, `fetch_past`, `past_from_heaps`, `scan_pages`, `fetch`/`walk`, `entries_from_heaps`) then aborts the transaction when the version naming an unreserved writer is gone under the exclusive latch.

This breaks storage.md §2 ("a page that changed before that latch is recorded as nothing and read again"), mvcc.md's deferrable read-only transaction that "never aborts with a serialization failure", and mvcc.md's rule that a serialization failure names its cause and the conflicting xids (here an empty list and a false cause). The catalog and history scans (catalog/mod.rs:482, :506, :1277, catalog/history.rs:356, catalog/kept.rs:182, history.rs:95) all `continue 'scan` on the same outcome. Reaching it needs damage (an unreserved writer) and a race, so it is latent outside the testing surface.

The same meaning of `Gone` is mapped by hand at four sites: heap/mod.rs:88, exec/mod.rs:176, write/mod.rs:248 (through `heap::Error::Changed`), vacuum/run.rs:30. write/reindex.rs:219 and write/verify.rs:475 refuse through `?` the same way.

Repro direction: plant an unreserved deleter (unreserved_tests' `with_unreserved_deleter`), remove the version between a scan's page read and `saw`, and expect rows, not `Serialization`.

Fix direction: return `Gone` to the per-page loops in store.rs and read the page again; delete exec's `From<Refused>` so no read path can turn `Gone` into an abort.

Found in the quality review of `reviewed..main` at 7bf40103.

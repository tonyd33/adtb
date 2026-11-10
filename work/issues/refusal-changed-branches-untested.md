---
created-at: 2026-11-10
status: open
kind: bug
priority: 550
summary: "the changed-under-latch branches of refuse_format, LargeStore::refuse_chunk and refuse_unreserved have no test that changes the page between the read and the latch"
tags:
  - quality
  - engine
  - test
  - class-weak-tests
---

# Bug: the changed-read-again branches of three refusals are untested

## Report

`refuse_format` (crates/adtb/src/heap/handle/mod.rs:362-375, looped in raw, read, scan_pages, preconvert, out_of_line, chain::stretch and rooted), `LargeStore::refuse_chunk` (crates/adtb/src/heap/large.rs:380-389) and `refuse_unreserved` / `names_writer` returning `Gone` (dead_slots at :964, sweep at :449) each read again or return `Gone` when the page changed before the exclusive latch. No test changes the page between the read and the latch for them; only `Version::looked_up` (unreserved_tests) and `Blame::refuse` (dir_tests) cover that branch.

Fix direction: one test per site that repairs the page between the read and the latch and asserts the read succeeds or retries with no damage recorded.

Found in the quality review of `reviewed..main` at 7bf40103.

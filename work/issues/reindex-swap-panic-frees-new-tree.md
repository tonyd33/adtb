---
created-at: 2026-10-30
status: open
kind: bug
priority: 432
summary: "a fatal I/O panic after the swap is caught as a failed rebuild, which frees the pages of the tree just swapped in"
tags:
  - quality
  - engine
---

# Reindex swap panic frees new tree

## Report

`crates/adtb/src/write/reindex.rs:308` catches every panic with text from `rebuild_and_swap` and treats it as a failed rebuild. A failed write or force is such a panic: `crates/adtb/src/storage/driver.rs:13` panics `fatal I/O error: ...` with a `String`.

`Pager::replace_contents` (`crates/adtb/src/storage/quarantine.rs`) forces the log and writes a blank root out after `update_all_with` has applied the swap in memory and removed the mark. A panic there unwinds out of `tree.take_over` at `write/reindex.rs:456`, before `made.clear()` and before `old.pages.extend` at 457-458. The `Err` arm at line 319 then:

- frees every page of the new tree through `free_built`, reached from a root that now names them;
- leaves the old pages out of `retire`;
- reports `Failure`, and a finding that says the old index was kept.

The index then names freed pages for the rest of the process, its mark is gone, and a fatal I/O error has become a job finding. `a_panic_after_the_swap_frees_no_page_of_the_new_index` injects its panic only at the `sweep` phase, after `made.clear()`.

Found by reading; not run. Repro direction: a sim fault failing the log force during the `swap` phase of `reindex`.

Fix direction: do not catch `fatal I/O error` panics as failures, and record the swap as done before anything after the in-memory change can unwind.

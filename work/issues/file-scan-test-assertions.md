---
created-at: 2026-10-26
status: open
kind: feature
priority: 770
summary: "file scan and load tests leave the whole-load abort, cell conversion failure, failed load atomicity and whole-row keys unpinned"
tags:
  - quality
  - test
---

# Feature: file scan and load tests pin what they claim

## Report

- `a_constraint_violation_aborts_the_whole_load_and_keeps_no_rejects` (`crates/adtb-sim/tests/files.rs:453-472`) checks that the rejects relation is empty but not that the target is, so the abort of the whole load is unpinned.
- The last item of `conformance/interop/03-file-scans.test`, a file whose content does not convert, reads `data/bad.csv` (header `id,user,gid`) at a row type of `user` and `bytes`: it fails on the missing `bytes` column, never on a cell conversion. Use a file with the right header and one bad cell.
- In `conformance/interop/04-loading.test`, the step where a load without a rejects relation fails with `BadInput` is followed by `delete i in identities where value(key i) >= 100`, which would also delete a partly applied row 210. Query `identities.Google` before the delete, expecting 10, 11 and 110.
- In `conformance/interop/05-temp-relations.test`, "without a key clause the whole row is the key" only counts an empty `pairs`. Insert two distinct rows (count 2) and a repeated row (a key violation), as `files.rs:623` does.

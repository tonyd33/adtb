---
created-at: 2026-11-07
status: open
kind: bug
priority: 140
summary: "the pager crash loop allows every Lost finding in a round with log trouble, while storage.md promises only that commits in the last unforced batch may be lost, so a loss outside that batch passes"
tags:
  - class-weak-tests
---

# pager crash loop tolerates any lost commit under any log trouble

## Report

## 2026-11-07

Found in review of test surface T6 (crates/adtb-sim/tests/storage/pager.rs, the `Reopened::allows` implementation). Pre-existing: main's loop returned before the check on the same condition. Direction: the allowance names the batch the log trouble hit, and the check allows `Lost` only for commits acknowledged in that batch; the loop asserts the allowed count is at most that batch's commits.

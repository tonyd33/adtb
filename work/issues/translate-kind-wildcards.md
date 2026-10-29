---
created-at: 2026-10-29
status: open
kind: bug
priority: 895
summary: "the translation walk's generic, compared and carries match Kind with wildcards, so a new kernel form keeps pinned ids or skips its checks silently"
tags:
  - quality
  - lang
---

# The translation walk matches Kind with wildcards

## Report

Three matches over `Kind` in `crates/adtb-lang/src/translate/` end in a wildcard:
- `generic` (`walk.rs:291`, `other => other`): a new form carrying a `RelId`, `FnId` or aggregate id keeps the pinned id and runs against the wrong object at the current version.
- `compared` (`walk.rs:368`, `_ => Ok(())`): a new form that compares, orders or deduplicates whole values skips the whole-value check.
- `carries` (`mod.rs:517`, `_ => {}`): a new form that passes a write's row through is not marked, so its missing fields are not filled.

Each compiles silently when a kernel form is added. `plan-binder-wildcards` and `bundle-reads-kind-wildcards` cover the same pattern elsewhere.

Fix: name every `Kind` in all three, with no wildcard.

---
created-at: 2026-11-04
status: open
kind: bug
priority: 640
summary: interop.md 6.3 says a restore fails with the violation reports; the engine reports one violation of least name as statements do
---

# Restore violation reports: one or all

## Report

## Report

interop.md §6.3: "If any uniqueness or constraint query finds a violation, the restore fails with the violation reports". The restore answers one `Violated` or `KeyViolation`, the one of least name (constraints.md §7), as the old one-change-set restore and every statement do. The plural reads as every report.

Found in the review of dump S5.3, 2026-11-04.

## Proposal

Through spec-revision: state that a restore fails with the violation of least name, as a statement does, unless a report of every violation is wanted, which would need a reason carrying a list.

---
created-at: 2026-10-29
status: open
kind: bug
priority: 1030
summary: "dormant grant rows found at open are reported nowhere, an older binary cannot open a file with a dormancy root row, and replay has no restore caller"
tags:
  - engine
---

# Bug: log grant rows follow-ups

## Report

From the two reviews of log-grant-rows (work/briefs/review-log-grant-rows{,2}.md, local).

- Dormant rows `record_grants` finds at open are dropped (crates/adtb/src/api/mod.rs around line 575); the ticket and security.md 2.1 say they are reported. Surface them (open result, audit or notice).
- An older binary fails to open a file holding a constructor-10 roots row (`Named::add` has no arm for it), against catalog.md 12.2's compatibility promise. Constructors 6 to 9 were added the same way; needs the upgrade rule for roots rows.
- `Catalog::replay` has no engine caller; it is the entry point restore (security.md 2.1) needs. Add the restore caller or a test-only marker.
- A carry that cannot carry a dormant grant of a missing role is refused, and only a revoke unblocks it; the refusal does not say so.
- Spec (spec-revision, catalog.md section 2): the `target` text form of a carried grant and the dormant activation ruling (log-grant-rows.md).

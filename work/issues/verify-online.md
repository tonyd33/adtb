---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 400
summary: "verify answers does not run in this engine yet: no Database::verify, no row-to-entry cross-reference, no logical check, no finding rows"
closed: 2026-10-30
---

# Feature: Verify online

## Request

Slice of milestone 9; scope, tests and acceptance in work/briefs/review-m9-audit.md, S1.

## Proposal

## Open questions

## 2026-10-30: landed

Landed as 1ca994cf. Full check passed: 2966 tests, 7 skipped; campaign 52. `verify` is refused in an
open transaction and in text that writes. Each run has its own `sys::jobs` row. `Overcounted` and
`Unsettled` are not findings; `Leaked` is. `adtb verify FILE` exits 1 on findings and 2 when it could
not run. Follow-ups are in verify-online-follow-ups.

## 2026-10-30: resolved

---
created-at: 2026-10-30
status: wontfix
kind: bug
priority: 560
summary: "dead versions of replaced findings accumulate in the roots heap, which no vacuum reaches"
closed: 2026-10-31
---

# Verify findings dead versions

## Report

Found by the verify-online track. Each verify run that replaces findings leaves dead versions in the heap of kept rows.

## 2026-10-31: closed by triage

Duplicate of verify-online-follow-ups.

## 2026-10-31: wontfix

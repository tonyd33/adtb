---
created-at: 2026-10-29
status: wontfix
kind: feature
priority: 500
summary: "only Grant audit events are written: registration, retirement and the other deployment commands security.md 8.1 lists are not audited"
closed: 2026-10-31
---

# Feature: Audit deployment events

## Request

Found by the checked-registration review (N3). security.md 8.1 lists bundle registration and retirement among audited events. `Tables::audit` always writes the `Grant` constructor, and no other event is written. Write `Register` from both registration paths, and the retirement and other deployment events, in the transaction of the change.

## 2026-10-31: closed by triage

Duplicate of audit-follow-ups.

## 2026-10-31: wontfix

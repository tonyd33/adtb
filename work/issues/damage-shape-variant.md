---
created-at: 2026-11-09
status: open
kind: feature
priority: 400
summary: "a page whose checksum passes but whose shape is wrong (a format byte past the newest, a foreign map root) is refused as Damage::Checksum, so damage reports cannot tell a torn write from a structural fault"
tags:
  - engine
---

# Feature: A typed damage variant for shape faults

## Request

Structural faults on a page that passes its checksum are refused as `Damage::Checksum`: heap's `refuse_format` (a format byte of 0 or past the heap's newest), the visibility-map root of the wrong kind or owner, and about five older heap sites the 2026-11-07 structural audit found. A report or a damage record then names a checksum failure for a page whose checksum held.

## Proposal

Add a damage variant for a page of the wrong shape (kind, owner, frame or format), carried by the damage record and the refusal reason, and use it at every shape-fault site. The record format change needs a ruling on compatibility of existing damage lists. Source: work/briefs/design-version-owner-r1-report.md Q3 (local); ruled out of structure-version-owner on 2026-11-08.

## Open questions

Whether the damage list's on-disk record gains a variant byte or a new record kind.

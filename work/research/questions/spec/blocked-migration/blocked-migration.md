---
title: "What a migration or revoke blocked by a live bundle does"
pick: one
options:
  refuse:
    title: "Refused at once with a report naming the blockers; BlockingMigration notices go to blocking sessions for BlockingNoticePeriod (24h)"
  wait:
    title: "Waits until the blocking bundle is released"
---
Raised by the third review of the spec in docs/spec/.

## spec/blocked-migration/refuse

Refused at once with a report naming the blockers; BlockingMigration notices go to blocking sessions for BlockingNoticePeriod (24h). Nothing waits, so no pending state and no ordering among waiting migrations.

## spec/blocked-migration/wait

Waits until the blocking bundle is released. Undefined held state; later migrations queue behind it.

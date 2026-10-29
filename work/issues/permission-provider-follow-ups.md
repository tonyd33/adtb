---
created-at: 2026-10-28
status: open
kind: bug
priority: 831
summary: "the permission provider renormalizes the program per statement, over-charges carried data in widened anonymous sums, and charges Rejects an insert on every destination constructor"
depends-on:
  - typed-permission-requirements
tags:
  - lang
  - quality
---

# Permission provider follow-ups

## Report

Found by the independent review of the typed permission requirements provider (work/briefs/review-permission-requirements.md, local).

- permission/mod.rs: `norm::program` runs again for every analyzed statement.
- check/adhoc.rs `retag_anon` builds fields without transport marks, so carried data in a widened anonymous sum is charged as read.
- permission/eval.rs: Rejects charges Insert on every constructor of the destination type, not only the constructors it can insert.

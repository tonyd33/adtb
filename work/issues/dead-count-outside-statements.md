---
created-at: 2026-11-04
status: open
kind: bug
priority: 580
summary: "Dead versions made by catalog writes, migrations and imports are not counted toward the background vacuum threshold"
---

# Dead versions outside statements go uncounted

## Report

The background vacuum's trigger counts dead versions from `Txn.changed`, which only the statement path (`statements_with`) fills. Catalog writes, migrations that rewrite rows, and imports make dead versions that are never counted, so those relations are vacuumed only once after open or when ordinary writes push them past the threshold.

Found in the review of background vacuum V3, 2026-11-04.

## Proposal

Count at the one place every write path passes through, the heap's delete and replace, rather than per statement. Then add a test: a migration that rewrites a relation's rows makes it due with no further writes.

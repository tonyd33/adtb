---
created-at: 2026-10-30
status: open
kind: bug
priority: 445
summary: a damaged chunk of a large value quarantines the database because every heap shares one store; 11.2 says one constructor relation
---

# Quarantine large values

## Report

Every heap shares one large-value store, so a bad chunk of a long `identities.Apple.sub` quarantines the database. Mapping needs a walk of row pointers or an owner per chunk. Fails safe; the blast radius is wider than ops/halt-scope/heap.

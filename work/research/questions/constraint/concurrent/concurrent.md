---
title: Concurrent transactions can jointly violate a cross-row constraint
pick: any
raised-by: [constraint/at-least-one, constraint/assertion, constraint/inclusion, constraint/cardinality, constraint/fd, constraint/exclusion]
options:
  serial-checks:
    title: Constraint checks' reads are validated serializably at any level
    links:
      - {rel: prior-art, to: prior/engine-pg-ssi}
  derived-locks:
    title: Locks derived from each delta check serialize conflicting writers
    needs: [constraint/enforce/delta]
    links:
      - {rel: prior-art, to: prior/constraint-uqjg}
      - {rel: prior-art, to: prior/engine-oracle-assertions}
  live:
    title: Live with constraints holding only under serializable transactions
    excludes: [constraint/concurrent/serial-checks, constraint/concurrent/derived-locks]
---
Two transactions each delete one of a user's two identities; each check
sees the other identity and passes; `has_identity` is violated
after both commit. Incremental checks assume isolation; Koppelaars,
Oriol and Teniente name this the main reason SQL assertions went
unimplemented. Unique and FK checks dodge it through index-level locks
on the probed key; general constraints probe more than one key.

## constraint/concurrent/serial-checks

Every index probe and scan a check makes enters the transaction's read
set as under `engine/ssi` or `engine/occ`, even when the transaction
runs at snapshot or read committed. A read-committed transaction whose
check conflicts aborts.

## constraint/concurrent/derived-locks

```
constraint has_identity = ...
-- delete i in identities  locks  (identities, user == i.user) shared-exclusive
```

Oracle 26ai assertion locks at table or join-column-value scope, held
to commit; validation refused at SERIALIZABLE. Only for constraints
with delta checks.

## constraint/concurrent/live

Weaker levels may commit violations; the spec states it.

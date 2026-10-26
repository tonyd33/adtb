---
title: "How catalog::audit is declared"
pick: one
options:
  seq:
    title: "relation catalog::audit : AuditEvent key seq"
---
Raised by checking the engine's own declarations. security.md §8.2
gives `AuditEvent`, whose header names `Key catalog::audit`, and the id
table of catalog.md §4.6 lists `catalog::audit`, but no block declared
the relation.

## spec/audit-relation/seq

```adt
relation catalog::audit : AuditEvent key seq
```

Its key is the sequence number every row carries.

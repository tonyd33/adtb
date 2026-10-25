---
title: What the engine audits
pick: any
options:
  auth:
    title: Authentication, grants, policy changes and bundle registration and retirement
  ops:
    title: Every schema operator appended to the log, with its author
  calls:
    title: Every registered query call with caller, bundle, query and parameters
  reads:
    title: Reads of constructor relations marked audited, with the keys returned
  writes:
    title: Every committed change set with its transaction's principal
---
The operator log already records every schema change; the redo log
records every write but not who asked.

## security/audit/auth

```
from e in audit.Grant where e.at > t select e
```

## security/audit/ops

The operator log gains an author field per operator.

## security/audit/calls

Registered-only access makes this a complete record of what
applications asked, since no other path exists for application roles.

## security/audit/reads

```
audit reads on patients.(Inpatient | Outpatient)
```

## security/audit/writes

Change sets are values (`write/surface/txdata`), so the logged record
is the change set itself.

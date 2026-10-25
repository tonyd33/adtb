---
title: How retag is privileged
pick: one
raised-by: [security/subject/relations]
options:
  crud:
    title: A retag needs delete on the source constructor and insert on the target
  transition:
    title: Retag privileges name source and target constructor sets
---
A retag is an atomic move between heaps (`union-index/retag/move`). In
a workflow sum, who may move `Draft` to `Sent` differs from who may
create `Sent`.

## security/transition/crud

```
grant delete invoices.Draft, insert invoices.Sent to clerk
```

The clerk can also create `Sent` invoices from nothing.

## security/transition/transition

```
grant retag invoices.Draft -> invoices.Sent to clerk
grant retag invoices.(Draft | Sent) -> invoices.Void to manager
```

The grants form a state machine over constructors. A `match` producing
a different constructor (`write/retag/assign`) is checked against the
pairs it can produce, at registration where the arms are known.

---
title: "Which constructor relations a statement reads, for quarantine and the wire"
pick: one
options:
  static-types:
    title: "The constructors in the types of the relation expressions of the normal form; parameters and pruning do not change it"
  after-pruning:
    title: "The constructors left after the planner prunes arms"
  with-parameters:
    title: "The constructors left once parameter values are known"
---
Raised by the tier audit. Quarantine and the wire's Arrow union were
stated "after pruning", so an observable outcome (a call fails, a union
lists a child) depended on what one planner prunes.

## spec/reads/static/static-types

The constructors a statement reads are those of the constructor sets
in the types of the relation expressions of its normal form. Parameter
values and planner pruning do not change it. Quarantine fails a call
exactly when that set includes a quarantined constructor relation. A
wire Arrow union lists the constructors of the static result type. The
`pruned` statistic is implementation-defined.

```adt
from i in identities where i.id == $k select i   -- reads EmailPass, Google, Apple
from g in identities.Google select g.gid         -- reads Google
```

## spec/reads/static/after-pruning

The set after the planner's pruning. Whether a call fails under
quarantine then depends on the planner, and changes when the planner
improves.

## spec/reads/static/with-parameters

The set once parameter values fix constructors. Needs its own rule for
which parameter uses fix a constructor, and the same call fails or
succeeds by argument.

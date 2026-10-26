---
title: "What StatementLimit says about the limit a statement exceeded"
pick: one
options:
  limit-sum:
    title: "StatementLimit carries limit: Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }"
  duration:
    title: "StatementLimit carries a Duration; other limits are reported otherwise"
---
Raised by `spec-memory-limit`. A call past its memory grant spills, and
when it cannot spill the statement is rejected; a fixpoint past its
round limit is rejected too. `StatementLimit { limit: Duration }` names
only a time limit.

## spec/limit-reason/limit-sum

```adt fragment
type Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }
| StatementLimit { limit: Limit }
```

`Time` is the statement limit or a declared limit, `Memory` a memory grant
the statement could not stay within, `Rounds` the fixpoint round
limit.

## spec/limit-reason/duration

Keep the `Duration` and add a reason per other limit. More
constructors of `RejectReason` for one kind of failure.

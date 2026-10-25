---
title: "Whether the serial order respects real time"
pick: one
options:
  primary:
    title: "On the primary, a transaction acknowledged before another began precedes it; replica reads keep their own guarantee"
  none:
    title: "No relation to real time"
---
Raised by the implementability review of docs/spec/. transactions §1.1
promised that a transaction beginning after another's acknowledged
commit sees it, but semantics §8.4 placed no such constraint on the
serial order.

## spec/txn/real-time/primary

The serial order extends real time on the primary: if `T`'s commit was
acknowledged before `T'` began, `T` precedes `T'`. Replica reads keep
the guarantee transactions §8 states for them.

## spec/txn/real-time/none

Any serial order. A client may fail to see its own acknowledged commit
on the primary.

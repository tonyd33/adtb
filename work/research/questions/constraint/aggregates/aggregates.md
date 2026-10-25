---
title: Bounded admission refuses every constraint with an aggregate
pick: any
raised-by: [constraint/admit/bounded]
options:
  maintained:
    title: Admit aggregate constraints whose aggregate the engine maintains per group
  refuse:
    title: Refuse aggregate constraints; users maintain totals as relations
  live:
    title: Live with no aggregate constraints at all
    excludes: [constraint/aggregates/maintained, constraint/aggregates/refuse]
---
"Every journal entry balances" sums its lines. An index-bounded delta
check of a sum needs the old total, so `admit/bounded` refuses it,
though it is the common case of a database-wide assertion.

## constraint/aggregates/maintained

```
constraint balanced =
  from e in entries
  where sum(l.amount for l in lines where l.entry == key e) != 0
  select e
```

admitted because the engine keeps the sum of `amount` per `entry` and updates
it from each change set, as an index is maintained. Count, sum, min and
max over a group key qualify; `min`/`max` need an index on the
aggregated field for deletes. Oracle 26ai refuses aggregates in
assertions (`kb/oracle-database.md`).

## constraint/aggregates/refuse

A total is a relation the application writes in the same change set,
checked by a row constraint.

## constraint/aggregates/live

No aggregates in constraints.

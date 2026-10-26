---
title: Where a constraint may read a transition relation
pick: one
raised-by: [constraint/admit/bounded, derived/transition/change-visible]
options:
  monotone-only:
    title: Refuse inserted or deleted under negation or inside an aggregate
  anywhere:
    title: Admit inserted and deleted in any position and check every constraint on every statement
---
A transition relation under negation makes a constraint fail on a
statement that touches nothing it reads.

```adt fragment
constraint once =
  from a in accounts
  where not exists d in inserted(accounts) where key d == key a
  select a
-- passes the statement that inserts account 1,
-- violated by the next statement, which touches nothing
```

## constraint/admit/transition-negation/monotone-only

Admission refuses a constraint in which `inserted` or `deleted` occurs
under negation or inside an aggregate, with a refusal naming the
occurrence. A change touching nothing a constraint reads then leaves
it empty.

## constraint/admit/transition-negation/anywhere

Every constraint that reads a transition relation in a non-monotone
position is checked on every statement, whatever it touches. Its cost
is no longer bounded by the change set.

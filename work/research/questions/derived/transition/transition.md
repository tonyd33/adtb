---
title: Whether a constraint can read the change as well as the state
pick: one
raised-by: [constraint/form/empty]
options:
  state-only:
    title: Constraints see only the state after the change
  change-visible:
    title: A constraint may read the inserted and deleted sets and the prior state
    links:
      - {rel: gains, to: constraint/enforce/delta}
  markers:
    title: Only declared markers such as append-only and immutable fields
---
`constraint/enforce/delta` already computes the change set; whether
the language lets a constraint name it decides whether append-only
logs, immutable fields and monotone counters are checkable.

## derived/transition/state-only

`events` cannot be declared append-only; a correction rewrites
history unseen.

## derived/transition/change-visible

```
constraint append_only = from e in deleted(events) select e
constraint no_reopen =
  from o in deleted(accounts.Closed), n in inserted(accounts.Open)
  where key o == key n
  select n
```

`inserted` and `deleted` are relations over the change set per
constructor relation, the form writes denote (`write/primitive/delta`);
a retag is a deletion from one constructor and an insertion into
another under one key. Checked with the same delta compiler.

## derived/transition/markers

```
relation events : Event key id append only
```

Fixed forms the engine checks directly.

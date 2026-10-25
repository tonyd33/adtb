---
title: How a retag appears to a change consumer
pick: one
raised-by: [repl/cdc/change-stream]
options:
  ctor-pair:
    title: A delete from the old constructor and an insert into the new, in one change set
  sum-update:
    title: One update event on the sum, carrying old and new constructor
---
`write/retag/assign` moves a row between heaps; consumers keyed by
sum key must not see a delete they act on before the insert.

## repl/cdc/retag/ctor-pair

Faithful to the delta primitive. A consumer subscribed to only one of
the two constructors sees half the move.

```
{ ctor: identities.EmailPass, deleted: [{ id: 9, ... }] }
{ ctor: identities.Google, inserted: [{ id: 9, ... }] }
```

## repl/cdc/retag/sum-update

The sum key is unique across constructors, so the pair is recognised
by key within a change set and folded into one event.

```
{ sum: identities, key: 9, before: EmailPass { ... }, after: Google { ... } }
```

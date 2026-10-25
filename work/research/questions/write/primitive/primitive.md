---
title: What a write denotes
pick: one
options:
  assign:
    title: Relational assignment, with insert, update and delete as shorthand
    links:
      - {rel: prior-art, to: prior/write-tutorial-d}
      - {rel: prior-art, to: prior/third-manifesto}
  delta:
    title: A pair of inserted and deleted relations per constructor
    links:
      - {rel: prior-art, to: prior/write-datomic-tx}
  rowwise:
    title: In-place modification of rows one at a time
---
The semantic core that `insert`, `update`, `delete`, retag and upsert
lower to. Constructor relations are the variables.

## write/primitive/assign

```
identities.Google := identities.Google union (from x in imports select Google { ... });
```

`delete g in identities.Google where p` is
`identities.Google := from g in identities.Google where not p select g`. A
retag assigns two constructor relations at once.

## write/primitive/delta

A write is `(plus, minus)` per constructor relation, computed against
the pre-state and applied together. Retag is a minus in `EmailPass` and
a plus in `Google` with one key. Incremental constraint checks and view
maintenance consume deltas directly.

## write/primitive/rowwise

Rows are modified in turn; a later row's predicate or value may see an
earlier row's write. MySQL evaluates single-table UPDATE assignments
left to right (unverified).

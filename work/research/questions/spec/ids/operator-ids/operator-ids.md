---
title: "How an operator records the ids of every object it creates"
pick: one
options:
  implied:
    title: "Only the first id is recorded; the rest follow from §1.2's order and the decl term"
  clause-rows:
    title: "A ClauseBody constructor Created { kind, id } lists every further id, one clause row each"
  operator-fields:
    title: "The Operator constructors carry the first id of each space they create in"
---
Raised by `id-assignment-gaps`. `AddType { sum, decl }`, `AddRelation
{ relation, decl }` and `Store { view, function, decl }` record one
id, though `add type` creates constructors and fields, `add relation`
indexes, defaults and computed fields, and `store` indexes.

## spec/ids/operator-ids/implied

A reader computes the other ids from the first, the decl term and the
counters of each space, which it must replay the log to know: the
first id of the field space is not in the operator.

## spec/ids/operator-ids/clause-rows

`ClauseBody` gains `Created { kind: Text, id: I64 }`, one row of
`catalog::migration_clauses` per further object in §1.2's order. The
version hash covers them as it covers every clause.

## spec/ids/operator-ids/operator-fields

An operator's objects of one space take consecutive ids, so the first
of each run and the decl term give them all. `AddType` gains
`first_field: Option I64`, `AddRelation` `first_index`,
`first_default` and `first_computed`, `Store` `first_index`. A list of
ids is not a field: fields hold no collections.

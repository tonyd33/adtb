---
title: "Which writes a let may bind"
pick: one
options:
  four-forms:
    title: "insert, upsert, update (new rows, fills applied) and delete (the rows it names); any other write is a type error"
  every-write:
    title: "Every write form, binding the rows of its change set"
---
Raised by the implementability review of docs/spec/. transactions §3.1
let `let x = w` bind "the rows the write would write" for any write, and
gave no type for `skip`, a write sequence, a write `if` or `match`, or
`fail`.

## spec/writes/let-write/four-forms

`let x = w` is defined for an insert (the inserted rows), an upsert (as
its binding is ruled), an update (the new rows, fills applied) and a
delete (the rows the write names, not those its delete actions derive).
A `let` over any other write form is a type error.

## spec/writes/let-write/every-write

Every write binds the rows of its change set. A sequence over several
relations has no row type, and `fail` and `skip` bind nothing useful.

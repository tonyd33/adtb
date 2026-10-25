---
title: "Which rows delete actions reach"
pick: one
options:
  pre-state:
    title: "Only pre-state rows; an inserted row referencing a gone key violates its reference, whatever the action"
  cascade-inserted:
    title: "cascade also reaches rows the statement inserts; retag and clear do not"
---
Raised by the implementability review of docs/spec/. semantics §6.4
let `cascade` reach a row the same statement inserts while `retag` and
`clear` did not, so one statement inserting a referrer and deleting its
target behaved differently by action.

## spec/writes/action-reach/pre-state

Delete actions reach only rows of the pre-state. A row the statement
inserts that references a gone key violates its reference constraint,
whatever the action. The statement says what it means by not inserting
the row.

## spec/writes/action-reach/cascade-inserted

`cascade` deletes inserted referrers as well. The actions disagree on
the same case, and a statement silently discards a row it wrote.

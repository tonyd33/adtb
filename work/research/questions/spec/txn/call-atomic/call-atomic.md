---
title: "What a call of several ad hoc items is in an interactive transaction"
pick: one
options:
  item-statements:
    title: "Each item is a statement; a call is atomic and its failure undoes all its statements, leaving the transaction open"
  call-statement:
    title: "The whole call is one statement"
---
Raised by the implementability review of docs/spec/. transactions §2
made an interactive transaction one statement per call while ad hoc
text of several items is one statement per item, so constraints held at
different boundaries for the same text inside and outside one; a
`transact` block sent inside an interactive transaction had no stated
commit behaviour.

## spec/txn/call-atomic/item-statements

In an interactive transaction each ad hoc item is a statement and
constraints hold after each. A call is atomic: if one of its statements
fails, every statement of that call is undone and the transaction stays
open. A `transact` block sent inside an interactive transaction never
commits on its own.

## spec/txn/call-atomic/call-statement

The whole call is one statement. The same text has different
constraint boundaries inside and outside an interactive transaction.

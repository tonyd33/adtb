---
title: "When a transaction may fail with a serialization failure"
pick: one
options:
  may:
    title: "Any transaction may be aborted; committed ones always have a serial order; read_only deferrable is the exception"
  only-cycle:
    title: "Only a transaction for which no serial order exists"
---
Raised by the implementability review of docs/spec/. transactions §6.1
said a transaction "whose commit would leave no serial order" is
aborted, which promises that no other transaction is.

## spec/txn/may-abort/may

A transaction may be aborted with a serialization failure when the
engine cannot place it in a serial order; one that commits always has
one. A `read_only deferrable` transaction is the stated exception.

## spec/txn/may-abort/only-cycle

Abort exactly the transactions for which no serial order exists. Rules
out every practical detector, which aborts some transactions that
could have committed.

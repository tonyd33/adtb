---
title: "What a resent commit of a vanished transaction returns"
pick: one
options:
  not-committed:
    title: "A definite not-committed error"
  expired:
    title: "The same error as an unknown transaction id"
---
Raised by the implementability review of docs/spec/. A client resending
the commit of an interactive transaction after a lost connection could
not tell a transaction that never committed and was since aborted from
an outcome still unknown.

## spec/txn/resend-unknown/not-committed

A resent commit of a transaction that never committed and no longer
exists fails with a definite not-committed error. Duplicates are found
by (client id, request number), and numbers below the client's
low-water mark are refused.

## spec/txn/resend-unknown/expired

The generic unknown-transaction error. The client cannot tell whether
to rerun the transaction.

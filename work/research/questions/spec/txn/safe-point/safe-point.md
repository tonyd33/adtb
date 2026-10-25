---
title: "What the spec says about safe snapshot formation"
pick: one
options:
  liveness:
    title: "A liveness promise bounded by SafePointDelay; the delaying mechanism leaves the spec"
  mechanism:
    title: "State that the primary delays new read-write transactions"
---
Raised by the tier review of docs/spec/. transactions §8 stated the
setting through the primary's delaying of new transactions.

## spec/txn/safe-point/liveness

Keep the bound as a promise: once no safe snapshot has formed for
`SafePointDelay`, one forms as soon as the read-write transactions
running then have ended, if they end within a further
`SafePointDelay`; a read-write transaction may start up to
`SafePointDelay` late for it. How the primary achieves this is
implementation.

## spec/txn/safe-point/mechanism

The spec states how the primary forces one. Names a mechanism, and
the observable promise is only implied.

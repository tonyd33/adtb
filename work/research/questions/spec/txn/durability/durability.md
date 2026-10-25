---
title: "What durability an interactive transaction commits at"
pick: one
options:
  strongest:
    title: "The strongest level named by its begin, its commit or any block run in it; deferrable inside is refused; read_only with durability is a type error"
  commit-only:
    title: "The level its commit names"
---
Raised by the implementability review of docs/spec/. A `transact`
block naming `durability quorum` sent inside an interactive transaction
had no stated effect, nor did `read_only deferrable` there, nor a block
declaring both `read_only` and a durability.

## spec/txn/durability/strongest

An interactive transaction's commit durability is the strongest level
named by its begin, its commit, or any block run in it. `deferrable`
inside an interactive transaction is refused. A block declaring both
`read_only` and `durability` is a type error.

## spec/txn/durability/commit-only

Only the commit's level counts. A block that asks for `quorum` is
silently acknowledged at `local`.

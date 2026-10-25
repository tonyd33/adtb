---
title: "How named queries, functions and transactions are declared"
pick: one
options:
  arrow:
    title: "Arrow before the result type"
  colon:
    title: "Colon before the result type"
---
Signatures are written (`lang/inference/signatures`).

## syntax/query-decl/arrow

`query emails (u: Key users) -> Set { email: Text } = ...`; `fn` for schema functions, `txn` for registered transactions; the result type is required.

## syntax/query-decl/colon

`query emails (u: Key users) : Set { email: Text } = ...`.

---
title: How errors reach clients
pick: one
options:
  closed-sum:
    title: Errors are values of closed prelude sums, matched exhaustively
  codes:
    title: Stable open error codes with structured detail
  host-only:
    title: Each host binding defines its own error types
---
Clients must tell a serialization failure (retry) from a constraint
violation (report) from a refused schema change. The constraint
violation already has a value: the violating rows of the denial query
(`constraint/form/empty`).

## project/spec/errors/closed-sum

```
type CommitError = SerializationFailure
                 | ConstraintViolated { constraint: Key catalog::constraints, rows: Bytes }
                 | ReaderRetired { bundle: Key catalog::bundles }
match r | SerializationFailure -> retry | ConstraintViolated v -> report(v) | ReaderRetired b -> redeploy
```

Wire-encoded by stable constructor id like any sum; codegen gives a
host enum.

## project/spec/errors/codes

SQLSTATE shape: five-character classes, new codes added freely; clients
branch on class.

## project/spec/errors/host-only

The Rust API returns Rust enums; other bindings map from the wire.

---
title: How errors are represented on the wire
pick: one
raised-by: [surface/wire/binary]
options:
  sum:
    title: Errors are values of a catalog sum, encoded with stable tags like any result
  code:
    title: A code string plus named fields, as SQLSTATE
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
      - {rel: prior-art, to: prior/net-cockroach}
---
Clients must tell retryable from final, and a constraint violation's
report is a query result (`constraint/form/empty`).

## net/errors/sum

```
type CallError =
    Serialization { retry_after: Option Duration }
  | Violation     { constraint: Name, rows: Bytes }      -- the report, typed by the constraint
  | Retired       { bundle: Name }
  | Cancelled | StreamIdle | Overloaded { retry_after: Duration }
  | Denied        { principal: Name, query: Name }
```

Codegen gives a host sum; adding a constructor follows readers-first
like any sum.

## net/errors/code

`40001 serialization_failure`; CockroachDB puts every retry kind under
40001 with a message. Fields are untyped strings.

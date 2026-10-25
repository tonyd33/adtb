---
title: Which time limits the engine enforces
pick: any
raised-by: [substrate/new-engine]
options:
  statement:
    title: A limit on each query or write execution
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  transaction:
    title: A limit on a transaction from begin to commit
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  idle:
    title: A limit on a transaction left open with nothing running
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  deadline:
    title: The caller sends a deadline with each call and the engine stops at it
    needs: [surface/wire/binary]
  declared:
    title: A registered query declares its own limit in its bundle
---
Postgres has statement, lock, idle-in-transaction, idle-session and
(since 17) transaction timeouts, all off by default. With SSI readers
never take locks, so `lock_timeout` has nothing to time beyond latch
and inserter waits.

## ops/limits/statement

Aborts the statement; the transaction can continue or roll back.

## ops/limits/transaction

Ends the transaction. With a retry block (settled) the client must not
retry a transaction killed by its own limit.

## ops/limits/idle

Ends a transaction idle between calls. An open transaction holds the
oldest-snapshot watermark (settled `mvcc/watermark`) and with it every
heap's dead versions.

## ops/limits/deadline

gRPC deadlines: absolute time in the request, so a timeout in the
application and in the engine are one number.

```
-> Execute { query: #7f3c, args: [..], deadline: 2026-10-25T10:00:01.250Z }
```

## ops/limits/declared

```
query report (m: Month) -> Set { region: Text, n: I64 } limit 30s =
  from o in orders where o.month == m
  group o by o.region into g select { region: g.key, n: count(x for x in g) }
```

The limit is part of the registered signature, checked and versioned
with the bundle.

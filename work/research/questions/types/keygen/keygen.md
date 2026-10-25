---
title: Which generators an engine-generated key field may use
pick: some
raised-by: [model/declared-key]
options:
  sequence:
    title: A per-relation integer counter
    links:
      - {rel: prior-art, to: prior/types-keygen}
  uuidv7:
    title: Time-ordered UUIDv7
    links:
      - {rel: prior-art, to: prior/types-keygen}
  uuidv4:
    title: Random UUIDv4
  snowflake:
    title: 64-bit time, node and per-node counter
    links:
      - {rel: prior-art, to: prior/types-keygen}
---
A generated key is a field with a default (`model/declared-key`),
visible and settable on import.

```
type User = { id: I64 = generate sequence, email: Email }
relation users : User key id
```

Generated keys land in a B-link tree; time-ordered values append at
the right edge, random ones scatter.

## types/keygen/sequence

Postgres sequences and Snowflake (the database) sequences: distinct
values, gaps after aborts, no gapless promise.

## types/keygen/uuidv7

RFC 9562: 48-bit millisecond timestamp, 74 random or counter bits;
monotone within a generator by method 1, 2 or 3 of sec. 6.2. Needs no
shared counter, so no conflict between inserters. Postgres 18
`uuidv7()`.

## types/keygen/uuidv4

122 random bits; no order, no leak.

## types/keygen/snowflake

Twitter: 41-bit milliseconds, 10-bit node, 12-bit sequence in an
`I64`. Needs node ids assigned once per server.

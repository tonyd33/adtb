---
title: Exclusion under a non-equality operator needs an index for it
pick: any
raised-by: [constraint/exclusion]
options:
  range-index:
    title: An interval index over a constructor set for overlap probes
    links:
      - {rel: prior-art, to: prior/constraint-pg-exclude}
  equality-prefix:
    title: Only equality fields plus one trailing range, probed per key
    links:
      - {rel: prior-art, to: prior/constraint-pg-exclude}
  live:
    title: Live with a scan of the scope per write
    excludes: [constraint/overlap/range-index, constraint/overlap/equality-prefix]
---
A unique check is an equality probe of a ctor-set index. "No two
bookings of a room overlap" needs "find rows whose range meets this
one", which a B-tree on `(room, during)` answers only by scanning the
room's rows.

## constraint/overlap/range-index

```
relation bookings : Booking
  key id
  interval index (resource, during) in (Room | Desk)
constraint no_double_booking =
  from a in bookings.(Room | Desk), b in bookings.(Room | Desk)
  where a.resource == b.resource and overlaps(a.during, b.during) and key a != key b
  select { a: key a, b: key b }
```

Postgres GiST, with entries `(key, ctor, tid)` like the union index.

## constraint/overlap/equality-prefix

SQL:2011 and Postgres 18 `WITHOUT OVERLAPS`: the B-tree on the
equality prefix finds the group; non-overlap of sorted ranges is a
neighbour check (unverified that Postgres does it this way; it uses
GiST).

## constraint/overlap/live

Cost grows with the scope's size.

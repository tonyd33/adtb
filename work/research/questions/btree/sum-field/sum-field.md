---
title: What indexes may be declared on value-level sum fields
pick: any
raised-by: [sum/value]
options:
  whole:
    title: Index the whole sum value
  by-tag:
    title: Index only the constructor tag
    links:
      - {rel: gains, to: btree/posting/posting-pairs}
  ctor-field:
    title: Index a payload field of one constructor, entries only for rows of it
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  match-expr:
    title: Index the value of a match expression over the field
---
A relation-level sum has a heap per constructor; a value-level sum is
a field, so its constructors share one heap. Indexes over it decide
what a key holds.

```
type Status = Draft | Sent { at: Time } | Paid { at: Time, ref: Text }
type Invoice = { status: Status }
relation invoices : Invoice key id
```

## btree/sum-field/whole

`index (status)` on `invoices`: key per `btree/sum-key`. Serves equality,
tag prefixes, and ranges if the sum is ordered.

## btree/sum-field/by-tag

`index (tag(status))` on `invoices`: one byte of key, at most as many
distinct keys as constructors, so a leaf is long runs of equal keys and
posting lists matter. With `whole` and `tag-payload` it is a prefix of
that index and adds nothing.

## btree/sum-field/ctor-field

```
relation invoices : Invoice
  key id
  unique (status.Paid.ref)
from i in invoices
where match i.status
  | Paid { ref } if ref == r -> true
  | Paid _ -> false
  | Sent _ -> false
  | Draft  -> false
select i.id
```

Entries only for rows whose `status` is `Paid`; a unique constraint
holds among them. Postgres partial unique index with an expression;
Postgres uses a partial index only when it proves the query's WHERE
implies its predicate.
https://www.postgresql.org/docs/current/indexes-partial.html

## btree/sum-field/match-expr

```
index (match status | Draft -> 0 | Sent _ -> 1 | Paid _ -> 2)   -- on invoices
```

A computed key, as Postgres expression indexes. Any total, pure match
is allowed; the planner matches the query's expression to the index's.

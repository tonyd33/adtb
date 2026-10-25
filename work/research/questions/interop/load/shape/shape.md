---
title: How flat external rows map onto a relation-level sum
pick: some
raised-by: [interop/load/source, interop/load/command, interop/load/append-api]
options:
  tag-match:
    title: A tag column routed to constructors by a match
    links:
      - {rel: variant-of, to: evolution/split-routing/match-route}
  per-ctor:
    title: One file per constructor, each inserted into its constructor relation
    links:
      - {rel: gains, to: storage/table-per-ctor}
  union-column:
    title: An Arrow dense union column maps member by member onto constructors
    links:
      - {rel: prior-art, to: prior/arrow-union}
  tagged-text:
    title: Each record carries its value in the textual value syntax
---
External formats have rows, not sums. A file of identities must say
which constructor each row is, and where the per-constructor fields
live.

## interop/load/shape/tag-match

```
insert identities from (
  from r in read_csv("identities.csv", IdentityRow)
  select match r.kind
    | "email"  -> EmailPass { id: r.id, user: Key users r.user, email: r.email, hash: r.hash }
    | "google" -> Google    { id: r.id, user: Key users r.user, gid: r.gid }
    | _        -> reject r)
```

The same routing match `evolution/split-routing/match-route` uses.
Per-constructor columns are absent or empty for other kinds; the arm
names only its own.

## interop/load/shape/per-ctor

```
transact
  insert identities.EmailPass from (from r in read_csv("email.csv", EmailRow) select ...)
  insert identities.Google    from (from r in read_parquet("google.parquet", GoogleRow) select ...)
```

One file per heap, as `EXPORT DATABASE` writes one file per table
(https://duckdb.org/docs/2.0/data/parquet/overview.html). Totality
and exclusivity hold because one `transact` makes it one change set.

## interop/load/shape/union-column

An Arrow IPC stream whose column is a dense union with one child per
constructor; child type id maps to constructor by name in the schema
metadata. Parquet has no union type (PARQUET-756), so only Arrow
sources take this path.

## interop/load/shape/tagged-text

```
{"id": 7, "user": 3, "identity": "Google { gid: \"g-123\" }"}
```

One text field parsed by `interop/text`.

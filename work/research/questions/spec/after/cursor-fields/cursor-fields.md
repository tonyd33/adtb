---
title: "Which field of an after cursor record corresponds to which ordering key"
pick: one
options:
  name-order:
    title: "The fields correspond to the ordering keys in the code-point order of their names"
  written-order:
    title: "The fields correspond to the ordering keys in the order they are written"
  key-names:
    title: "Each field is named after its ordering key"
---
Raised while implementing the checker against docs/spec/. queries.md
§14.4 says the fields of the cursor record of `after`, for a list of
ordering keys, "are compared positionally". Record types are
unordered, so a record has no positions.

```adt
query feed_page (cursor: Option ({ at: Instant, id: Key posts })) -> Set Post =
  from p in posts select p order by (p.created_at desc, key p) take 20 after cursor
```

## spec/after/cursor-fields/name-order

The cursor record's fields correspond to the ordering keys in the
code-point order of their names: the first name to the first key, and
so on. The names are otherwise free. `{ at, id }` pairs `at` with
`p.created_at` and `id` with `key p`.

## spec/after/cursor-fields/written-order

The fields correspond to the keys in the order the record type is
written. `{ a: X, b: Y }` and `{ b: Y, a: X }` are one type, so the
order is not a property of the type, and a parameter type that
arrives through a descriptor has no written order.

## spec/after/cursor-fields/key-names

Each field takes a name derived from its ordering key. Keys are
expressions, often with no name (`key p`, `p.a + p.b`), so a naming
rule for every expression is needed.

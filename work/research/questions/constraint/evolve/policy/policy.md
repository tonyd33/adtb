---
title: What adding a constraint does to rows that violate it
pick: some
raised-by: [constraint/evolve/icmo]
options:
  check:
    title: Refuse the operator while any row violates the constraint
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
  repair:
    title: Delete or move violating rows, a declared loss
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
  novalidate:
    title: Enforce on new writes only, existing rows unchecked
    links:
      - {rel: prior-art, to: prior/constraint-sql-assertion}
---
PRISM++ `CHECK` and `ENFORCE`; Oracle `ENABLE NOVALIDATE`.

## constraint/evolve/policy/check

```
add constraint primary_owned = ...
-- refused: 3 violations, query primary_owned to list them
```

Information preserving, so its inverse is a drop.

## constraint/evolve/policy/repair

```
add constraint no_double_booking = ... repair into bookings.Conflict
```

PRISM++ `ENFORCE` deletes violators. Moving them to a constructor keeps
the data; the operator is lossy either way for the old schema.

## constraint/evolve/policy/novalidate

The constraint is no longer a fact about the database: a refinement
type or an optimizer rewrite that relies on it is wrong on old rows.

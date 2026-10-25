---
title: Which SQL encodings of sums the migration recognises
pick: any
raised-by: [interop/migrate/tool, interop/migrate/attach]
options:
  nullable-option:
    title: A nullable column becomes an Option field
  sti:
    title: A type column with per-type nullable columns becomes a relation-level sum
    links:
      - {rel: prior-art, to: prior/interop-sql-patterns}
  pg-inherit:
    title: Postgres inheritance children become constructors of one sum
    links:
      - {rel: prior-art, to: prior/interop-sql-patterns}
  polymorphic:
    title: A type plus id pair becomes a sum with one reference per constructor
    links:
      - {rel: prior-art, to: prior/interop-sql-patterns}
  exclusive-arc:
    title: Several nullable FKs with an exactly-one check become a sum
  jsonb-tagged:
    title: A jsonb column with a type field becomes a value-level sum
---
Each pattern is a mapping rule the tool proposes and the user
confirms. `encoding/wide`, `encoding/tag-variants` and
`encoding/jsonb` are these patterns, defeated as targets.

## interop/migrate/patterns/nullable-option

`phone text NULL` becomes `phone: Option Text`. See
`interop/migrate/null`.

## interop/migrate/patterns/sti

```
-- source: vehicles(id, type, wheels NULL, sail_area NULL)
type Vehicle = Car { wheels: I32 } | Boat { sail_area: F64 }
relation vehicles : Vehicle key id
```

Rails STI stores the subclass name in `type`
(https://guides.rubyonrails.org/association_basics.html). Rows whose
other-type columns are set fail the load (`interop/migrate/dirty`).

## interop/migrate/patterns/pg-inherit

`capitals INHERITS (cities)` becomes `type City = Plain { ... } | Capital { ... }`.
Postgres never enforced uniques or FKs across children
(https://www.postgresql.org/docs/current/ddl-inherit.html), so the
first load may find duplicates.

## interop/migrate/patterns/polymorphic

```
-- source: pictures(id, imageable_type, imageable_id)
type Picture = OfEmployee { owner: Key employees } | OfProduct { owner: Key products }
relation pictures : Picture key id
```

References live only at relation level (`model/refs-top-level`), so
the sum is relation-level and each constructor has its own FK. Rails
cannot declare an FK for `imageable_id` (unverified).

## interop/migrate/patterns/exclusive-arc

`(employee_id NULL, product_id NULL, CHECK (num_nonnulls(..) = 1))`
maps to the same sum as `polymorphic`.

## interop/migrate/patterns/jsonb-tagged

`payload jsonb` whose objects all carry `"kind"` becomes a value-level
sum through `interop/json-map/internal`; the tool samples or scans
every row for the set of kinds and their fields.

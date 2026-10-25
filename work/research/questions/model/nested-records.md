---
title: Whether a field may hold a record
pick: one
options:
  inline-records:
    title: A field's type may be any record type and is stored inline
    links:
      - {rel: prior-art, to: prior/lang-nrc}
  flat-fields:
    title: Fields hold scalars and value-level sums only
---
Value-level sums already nest records as constructor payloads
(`US { zip: Text }`). Whether a bare record may be a field.

## model/inline-records

```
type Order = { ship: { street: Text, city: Text }, bill: Address }
relation orders : Order key id
from o in orders select o.ship.city
```

DuckDB STRUCT (https://duckdb.org/docs/sql/data_types/struct), NRC.

## model/flat-fields

A grouped field is either flattened (`ship_street`, `ship_city`) or a
one-constructor value sum, which under `model/record-is-ctor` is the
same thing as a record.

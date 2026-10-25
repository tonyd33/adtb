---
title: How a write changes a value-level sum inside a row
pick: some
raised-by: [sum/value]
options:
  replace:
    title: The field is assigned a whole new value
  path:
    title: A path through a constructor sets one inner field
    links:
      - {rel: variant-of, to: model/nested-update/path-update}
      - {rel: prior-art, to: prior/write-pg-composite}
      - {rel: prior-art, to: prior/write-optics}
  rebuild:
    title: A match rebuilds the value, copying the arm with one field changed
---
`type Customer = { address: Address }` with `type Address = US { zip:
Text, street: Text } | Intl { country: Text, postcode: Text }`. Constructor fields are not keys, so no
index or FK constrains an inner write.

## write/nested/replace

```
update c in customers where c.id == k
set Customer { c with address: US { zip: "02139", street: "1 Main" } }
```

The only form needed; every other form is sugar for it.

## write/nested/path

```
update c in customers where c.id == k set { address.US.zip: "02139" }
```

Postgres `SET complex_col.r = ...` for composites; a prism setter for a
sum.

## write/nested/rebuild

```
update c in customers where c.id == k
set Customer { c with address: match c.address
  | US u   -> US { u with zip: "02139" }
  | Intl i -> i }
```

Exhaustive, so a new constructor forces every such update to say what
it does.

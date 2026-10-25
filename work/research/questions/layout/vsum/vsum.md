---
title: How a value-level sum field is stored inside a row
pick: one
raised-by: [sum/value]
options:
  max-payload:
    title: Tag plus a payload area sized to the largest constructor
    links:
      - {rel: prior-art, to: prior/layout-union-encoding}
  tag-var:
    title: Tag in the fixed part and the payload as a variable-width field
  flattened:
    title: Tag plus a slot for every field of every constructor
    links:
      - {rel: prior-art, to: prior/arrow-union}
      - {rel: prior-art, to: prior/duckdb-union}
  by-shape:
    title: Max-payload when every constructor is fixed width and small, tag-var otherwise
---
A row has one schema, but a field of sum type has one of several
payload shapes. Nullary constructors (enums, `None`) have no payload.

```
type Status  = Active | Suspended { until: Instant } | Deleted
type Address = US { zip: Text } | Intl { country: Code2, postal: Text }
type User    = { id: I64, status: Status, addr: Address }
relation users : User key id
```

## layout/vsum/max-payload

Rust enums and C tagged unions; Cap'n Proto overlaps union members in
the data section under a 16-bit discriminant.

```
status:  u8 tag | pad 7 | i64 until    16 B, Active and Deleted leave 8 B unused
```

Fixed width when every payload is, so `layout/slots/fixed-slot` still
applies. Nested sums nest the rule: width is the maximum over the tree.

## layout/vsum/tag-var

```
fixed part:  status: u8 tag      addr: u8 tag
var part:    status payload (0 or 8 B)   addr payload (len-prefixed fields)
```

Nullary constructors cost only the tag. Payload located by
`layout/var-fields`.

## layout/vsum/flattened

Arrow sparse union inside a row: `status_tag, until`; `addr_tag,
us_zip, intl_country, intl_postal`. Fields of inactive constructors are
dead bytes; a columnar layout can store each as its own column.

## layout/vsum/by-shape

`Status` (payloads 0, 8, 0 B) is max-payload, 9 to 16 B; `Address`
(holding `Text`) is tag-var. The threshold is an engine constant.

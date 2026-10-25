---
title: A value-level sum field has no native column in the format
pick: any
raised-by: [interop/formats/csv, interop/formats/parquet]
options:
  text-cell:
    title: The field is one text cell in the textual value syntax
  tag-columns:
    title: A tag column plus one column or struct per constructor, empty for the others
    links:
      - {rel: prior-art, to: prior/duckdb-union}
  live:
    title: Live with sum fields unexportable to these formats
    excludes: [interop/formats/flat-sums/text-cell, interop/formats/flat-sums/tag-columns]
---
`type Contact = { name: Text, via: Email { addr: Text } | Phone { num: Text } }`
written to CSV or Parquet.

## interop/formats/flat-sums/text-cell

```
name,via
ann,"Email { addr: ""a@b"" }"
```

Round-trips exactly through `interop/text`; unreadable as columns by
other tools.

## interop/formats/flat-sums/tag-columns

```
name,via_tag,via_email_addr,via_phone_num
ann,Email,a@b,
```

Parquet: `via: struct<tag: string, Email: struct<addr>, Phone: struct<num>>`,
the DuckDB UNION layout as a struct
(https://duckdb.org/docs/2.0/sql/data_types/union.html). The reader
checks that exactly the tagged member is present.

## interop/formats/flat-sums/live

Only relation-level sums export, one file per constructor.

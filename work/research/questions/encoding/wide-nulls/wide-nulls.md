---
title: Wide table fields are nullable columns guarded by CHECKs
pick: any
raised-by: [encoding/wide]
options:
  generated-checks:
    title: Compiler generates the CHECK and partial indexes, users never see nulls
  live:
    title: Live with nullable columns and row width
    excludes: [encoding/wide-nulls/generated-checks]
---
Every constructor's fields are columns of every row. Column catalog says
nullable; a reader of the table cannot see constructor membership
without the CHECK. FKs from a constructor field must be declared on a
nullable column. Width grows with the sum of all constructors' fields.

## encoding/wide-nulls/generated-checks

The schema language owns the table; constructor views expose each
constructor with NOT NULL semantics.

## encoding/wide-nulls/live

Accept nullable catalog columns and wide rows.

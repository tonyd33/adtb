---
title: How query parameters are typed
pick: one
options:
  declared:
    title: Parameters declared with types in the query signature
  inferred:
    title: Parameter types inferred from use
  spliced:
    title: Host values spliced into a quoted query
    needs: [lang/registered/extracted]
    links:
      - {rel: prior-art, to: prior/lang-links}
---
Prepared and registered queries take parameters. A parameter of a sum
type (`status: Status`) is a sum value like any other; none is NULL.

## lang/params/declared

```
query by_status (s: Status, since: Time) -> Set Invoice =
  from i in invoices where i.status == s and i.issued >= since select i
```

## lang/params/inferred

```
query by_status = from i in invoices where i.status == $s select i   -- $s : Status
```

Postgres infers `$1` types at `PREPARE`. An ambiguous use is an error.

## lang/params/spliced

T-LINQ antiquotation: the host value's type is the parameter's type,
checked by the host compiler against the schema.
